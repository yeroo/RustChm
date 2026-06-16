//! Full-text search index ($FIftiMain) writer. Port of FastChm's fifti.cpp.

use std::collections::BTreeMap;

use crate::bytebuf::{encint_len, Buf};

const NODE: usize = 4096;
const R_DOC: u32 = 2;
const R_CODE: u32 = 1;
const R_LOC: u32 = 5;

fn lower(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        c + 32
    } else {
        c
    }
}

struct TextRun {
    text: Vec<u8>,
    title: bool,
}

fn decode_entity(ent: &str) -> Vec<u8> {
    match ent {
        "amp" => b"&".to_vec(),
        "lt" => b"<".to_vec(),
        "gt" => b">".to_vec(),
        "quot" => b"\"".to_vec(),
        "apos" => b"'".to_vec(),
        "nbsp" => b" ".to_vec(),
        _ if ent.starts_with('#') => {
            let hex = ent.len() > 1 && (ent.as_bytes()[1] | 0x20) == b'x';
            let num = if hex { &ent[2..] } else { &ent[1..] };
            if let Ok(code) = u32::from_str_radix(num, if hex { 16 } else { 10 }) {
                if code > 0 && code < 256 {
                    return vec![code as u8];
                }
            }
            b" ".to_vec()
        }
        _ => b" ".to_vec(),
    }
}

fn extract_text(html: &[u8]) -> Vec<TextRun> {
    let s = html;
    let mut runs: Vec<TextRun> = Vec::new();
    let (mut in_title, mut in_body, mut in_script, mut in_style) = (false, false, false, false);
    let mut cur: Vec<u8> = Vec::new();
    let mut cur_title = false;
    let flush = |cur: &mut Vec<u8>, runs: &mut Vec<TextRun>, title: bool| {
        if !cur.is_empty() {
            runs.push(TextRun { text: std::mem::take(cur), title });
        }
    };
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'<' {
            if s[i..].starts_with(b"<!--") {
                i = s[i..].windows(3).position(|w| w == b"-->").map(|x| i + x + 3).unwrap_or(s.len());
                continue;
            }
            let close = match s[i..].iter().position(|&c| c == b'>') {
                Some(x) => i + x,
                None => break,
            };
            let mut tag = String::new();
            let mut k = i + 1;
            while k < close && tag.len() < 8 {
                tag.push(lower(s[k]) as char);
                k += 1;
            }
            flush(&mut cur, &mut runs, cur_title);
            if in_body {
                if tag.starts_with("/body") {
                    in_body = false;
                } else if tag.starts_with("script") {
                    in_script = true;
                } else if tag.starts_with("/script") {
                    in_script = false;
                } else if tag.starts_with("style") {
                    in_style = true;
                } else if tag.starts_with("/style") {
                    in_style = false;
                }
            } else if tag.starts_with("title") {
                in_title = true;
            } else if tag.starts_with("/title") {
                in_title = false;
            } else if tag.starts_with("body") {
                in_body = true;
            }
            i = close + 1;
            continue;
        }
        let c = s[i];
        if c == b'&' {
            if let Some(semi) = s[i..].iter().position(|&x| x == b';') {
                let semi = i + semi;
                if semi - i <= 10 {
                    let ent = std::str::from_utf8(&s[i + 1..semi]).unwrap_or("");
                    if (in_title && !in_body) || (in_body && !in_script && !in_style) {
                        cur_title = in_title && !in_body;
                        cur.extend_from_slice(&decode_entity(ent));
                    }
                    i = semi + 1;
                    continue;
                }
            }
        }
        if (in_title && !in_body) || (in_body && !in_script && !in_style) {
            cur_title = in_title && !in_body;
            cur.push(c);
        }
        i += 1;
    }
    flush(&mut cur, &mut runs, cur_title);
    runs
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

struct BitWriterMsb {
    out: Vec<u8>,
    buf: u8,
    used: i32,
}
impl BitWriterMsb {
    fn new() -> Self {
        BitWriterMsb { out: Vec::new(), buf: 0, used: 0 }
    }
    fn put(&mut self, value: u32, nbits: i32) {
        for b in (0..nbits).rev() {
            self.buf = (self.buf << 1) | ((value >> b) & 1) as u8;
            self.used += 1;
            if self.used == 8 {
                self.out.push(self.buf);
                self.buf = 0;
                self.used = 0;
            }
        }
    }
    fn align_byte(&mut self) {
        if self.used != 0 {
            self.put(0, 8 - self.used);
        }
    }
}

fn bitlen(mut v: u32) -> i32 {
    let mut n = 0;
    while v != 0 {
        n += 1;
        v >>= 1;
    }
    n
}

fn sr_put(bw: &mut BitWriterMsb, v: u32, root: i32) {
    let m = bitlen(v);
    if m <= root {
        bw.put(0, 1);
        bw.put(v, root);
    } else {
        bw.put(0xFFFF_FFFF, m - root); // prefix of (m-root) ones
        bw.put(0, 1);
        bw.put(v - (1u32 << (m - 1)), m - 1);
    }
}

fn encint_le(out: &mut Vec<u8>, mut v: u32) {
    let mut groups = [0u8; 5];
    let mut n = 0;
    loop {
        groups[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in 0..n {
        out.push(groups[i] | if i + 1 < n { 0x80 } else { 0 });
    }
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let mut n = 0;
    while n < a.len() && n < b.len() && a[n] == b[n] {
        n += 1;
    }
    n
}

#[derive(Default)]
struct NodeBuf {
    content: Vec<u8>,
    last_word: Vec<u8>,
}
impl NodeBuf {
    fn used(&self) -> bool {
        !self.content.is_empty()
    }
}

struct FiftiWriter {
    file: Buf,
    leaf: NodeBuf,
    levels: Vec<NodeBuf>,
    leaf_count: u32,
    prev_leaf_start: u32,
    last_leaf_free: u32,
}

impl FiftiWriter {
    fn new() -> Self {
        let mut file = Buf::new();
        file.zeros(0x400);
        FiftiWriter {
            file,
            leaf: NodeBuf::default(),
            levels: Vec::new(),
            leaf_count: 0,
            prev_leaf_start: u32::MAX,
            last_leaf_free: 0,
        }
    }

    fn add_word(&mut self, word: &[u8], context: i32, docs: &BTreeMap<u32, Vec<u32>>) {
        let mut wlc = BitWriterMsb::new();
        let mut last_doc = 0u32;
        for (&doc, locs) in docs {
            sr_put(&mut wlc, doc - last_doc, R_DOC as i32);
            last_doc = doc;
            sr_put(&mut wlc, locs.len() as u32, R_CODE as i32);
            let mut last_loc = 0u32;
            for &loc in locs {
                sr_put(&mut wlc, loc - last_loc, R_LOC as i32);
                last_loc = loc;
            }
            wlc.align_byte();
        }
        let wlc_bytes = wlc.out;

        let entry_size = |base: &[u8]| -> usize {
            let part = word.len() - common_prefix(word, base);
            2 + part + 1 + encint_len(docs.len() as u64) + 6 + encint_len(wlc_bytes.len() as u64)
        };
        if self.leaf.used() && 8 + self.leaf.content.len() + entry_size(&self.leaf.last_word) > NODE {
            self.flush_leaf(true);
        }
        let wlc_offset = self.file.len() as u32;
        self.file.raw(&wlc_bytes);

        let prefix = if self.leaf.used() {
            common_prefix(word, &self.leaf.last_word)
        } else {
            0
        };
        let c = &mut self.leaf.content;
        c.push((word.len() - prefix + 1) as u8);
        c.push(prefix as u8);
        c.extend_from_slice(&word[prefix..]);
        c.push(context as u8);
        encint_le(c, docs.len() as u32);
        for k in 0..4 {
            c.push((wlc_offset >> (k * 8)) as u8);
        }
        c.push(0);
        c.push(0);
        encint_le(c, wlc_bytes.len() as u32);
        self.leaf.last_word = word.to_vec();
    }

    fn finish(&mut self) -> (u32, u32, u16, u32) {
        if self.leaf.used() {
            self.flush_leaf(false);
        }
        let nlevels = self.levels.len();
        for lv in 0..nlevels {
            self.flush_index_node(lv, lv + 1 < nlevels);
        }
        let root_offset = self.file.len() as u32 - NODE as u32;
        (root_offset, self.leaf_count, (1 + nlevels) as u16, self.last_leaf_free)
    }

    fn flush_leaf(&mut self, _more: bool) {
        let my_start = self.file.len() as u32;
        if self.prev_leaf_start != u32::MAX {
            for k in 0..4 {
                self.file.v[self.prev_leaf_start as usize + k] = (my_start >> (k * 8)) as u8;
            }
        }
        self.prev_leaf_start = my_start;
        let free = (NODE - 8 - self.leaf.content.len()) as u32;
        self.last_leaf_free = free;
        let mut h = Buf::new();
        h.u32(0); // next leaf (patched later)
        h.u16(0);
        h.u16(free as u16);
        self.file.raw(&h.v);
        let content = std::mem::take(&mut self.leaf.content);
        let last_word = std::mem::take(&mut self.leaf.last_word);
        self.file.raw(&content);
        self.file.zeros(free as usize);
        self.leaf_count += 1;
        // register in the bottom index level when the tree needs one
        let more = _more;
        if more || !self.levels.is_empty() {
            self.add_index_entry(0, &last_word, my_start);
        }
        self.leaf = NodeBuf::default();
    }

    fn add_index_entry(&mut self, level: usize, word: &[u8], child_offset: u32) {
        if level >= self.levels.len() {
            self.levels.push(NodeBuf::default());
        }
        let mut prefix = if self.levels[level].used() {
            common_prefix(word, &self.levels[level].last_word)
        } else {
            0
        };
        if 2 + self.levels[level].content.len() + (2 + word.len() - prefix + 6) > NODE {
            self.flush_index_node(level, true);
            prefix = 0;
        }
        let node = &mut self.levels[level];
        let c = &mut node.content;
        c.push((word.len() - prefix + 1) as u8);
        c.push(prefix as u8);
        c.extend_from_slice(&word[prefix..]);
        for k in 0..4 {
            c.push((child_offset >> (k * 8)) as u8);
        }
        c.push(0);
        c.push(0);
        node.last_word = word.to_vec();
    }

    fn flush_index_node(&mut self, level: usize, register_in_parent: bool) {
        if !self.levels[level].used() {
            return;
        }
        let my_start = self.file.len() as u32;
        let last_word = self.levels[level].last_word.clone();
        if register_in_parent {
            self.add_index_entry(level + 1, &last_word, my_start);
        }
        let content = std::mem::take(&mut self.levels[level].content);
        let free = (NODE - 2 - content.len()) as u16;
        let mut h = Buf::new();
        h.u16(free);
        self.file.raw(&h.v);
        self.file.raw(&content);
        self.file.zeros(free as usize);
        self.levels[level] = NodeBuf::default();
    }
}

#[derive(Default)]
pub struct FtsIndexer {
    // (word, context) -> topic -> positions
    words: BTreeMap<(Vec<u8>, i32), BTreeMap<u32, Vec<u32>>>,
    file_count: u32,
    total_words: u64,
    total_word_len: u64,
    longest_word: u32,
}

impl FtsIndexer {
    pub fn new() -> Self {
        FtsIndexer::default()
    }

    pub fn has_data(&self) -> bool {
        self.file_count > 0 && !self.words.is_empty()
    }

    pub fn index_file(&mut self, html: &[u8], topic_index: u32) {
        let runs = extract_text(html);
        if runs.is_empty() {
            return;
        }
        self.file_count += 1;
        let mut pos = 0u32;
        for run in &runs {
            let t = &run.text;
            let mut i = 0;
            while i < t.len() {
                let c0 = lower(t[i]);
                if !is_word_char(c0) {
                    i += 1;
                    continue;
                }
                let number_word = c0.is_ascii_digit();
                let mut word: Vec<u8> = Vec::new();
                while i < t.len() {
                    let c = lower(t[i]);
                    if is_word_char(c) {
                        word.push(c);
                    } else if number_word && c == b'.' && !word.is_empty() {
                        word.push(c);
                    } else if c == b'\'' && !word.is_empty() {
                        // elided
                    } else {
                        break;
                    }
                    i += 1;
                }
                if word.len() <= 99 {
                    let key = (word.clone(), if run.title { 1 } else { 0 });
                    self.words.entry(key).or_default().entry(topic_index).or_default().push(pos);
                    self.total_words += 1;
                    self.total_word_len += word.len() as u64;
                    self.longest_word = self.longest_word.max(word.len() as u32);
                }
                pos += 1;
            }
        }
    }

    pub fn build(&self, lcid: u32, codepage: u32) -> Vec<u8> {
        if !self.has_data() {
            return Vec::new();
        }
        let mut w = FiftiWriter::new();
        let mut unique_len = 0u64;
        for ((word, ctx), docs) in &self.words {
            w.add_word(word, *ctx, docs);
            unique_len += word.len() as u64;
        }
        let (root_offset, leaf_count, tree_depth, last_leaf_free) = w.finish();

        let mut h = Buf::new();
        h.u8(0);
        h.u8(0);
        h.u8(0x28);
        h.u8(0);
        h.u32(self.file_count);
        h.u32(root_offset);
        h.u32(0);
        h.u32(leaf_count);
        h.u32(root_offset);
        h.u16(tree_depth);
        h.u32(7);
        h.u8(2);
        h.u8(R_DOC as u8);
        h.u8(2);
        h.u8(R_CODE as u8);
        h.u8(2);
        h.u8(R_LOC as u8);
        h.zeros(10);
        h.u32(NODE as u32);
        h.u32(0);
        h.u32(1);
        h.u32(5);
        h.u32(self.longest_word);
        h.u32(self.total_words as u32);
        h.u32(self.words.len() as u32);
        h.u32(self.total_word_len as u32);
        h.u32(0);
        h.u32(unique_len as u32);
        h.u32(last_leaf_free);
        h.u32(0);
        h.u32(self.file_count - 1);
        h.zeros(24);
        h.u32(codepage);
        h.u32(lcid);
        let mut file = w.file.v;
        file[..h.len()].copy_from_slice(&h.v);
        file
    }
}
