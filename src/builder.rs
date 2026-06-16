//! Compiles an HTML Help project (.hhp) into a .chm. Full port of FastChm's
//! builder.cpp: auto-inclusion, sitemap, binary TOC/index, KLinks/ALinks,
//! window definitions, context IDs, merge files, subsets, full-text search,
//! codepage/Unicode handling, and collection builds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::bytebuf::Buf;
use crate::chmwriter::{write_container, DirEntry};
use crate::fifti::FtsIndexer;
use crate::lzx::lzx_compress;
use crate::objinst_data::OBJINST_CHAR_TABLE;
use crate::sitemap::{parse_sitemap, scan_link_objects, SiteMap, SiteMapItem};
use crate::textenc::{
    append_utf16le, codepage_for_lcid, decode_auto, decode_text, encode_codepage,
    is_dbcs_codepage,
};

pub struct Stats {
    pub file_count: usize,
    pub uncompressed: u64,
    pub compressed: u64,
    pub output: u64,
}

fn slashes(s: &str) -> String {
    s.replace('\\', "/")
}
fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}
fn trim(s: &str) -> &str {
    s.trim()
}

// ---------------- HHP project ----------------

#[derive(Default)]
struct Project {
    dir: String,
    options: HashMap<String, String>,
    files: Vec<String>,
    window_lines: Vec<String>,
    merge_files: Vec<String>,
    subsets: Vec<String>,
    info_types: Vec<String>,
    aliases: Vec<(String, String)>,    // name -> file
    map_defs: Vec<(String, u32)>,      // name -> context id
}

impl Project {
    fn opt(&self, k: &str) -> String {
        self.options.get(k).cloned().unwrap_or_default()
    }
    fn opt_yes(&self, k: &str) -> bool {
        matches!(lower(&self.opt(k)).as_str(), "yes" | "true" | "1")
    }
}

fn parse_alias_line(line: &str, dir: &str, p: &mut Project) {
    if line.starts_with('#') {
        let mut inc = trim(&line[line.find([' ', '\t']).map(|x| x + 1).unwrap_or(line.len())..]).to_string();
        if inc.starts_with('"') {
            inc = inc.trim_matches('"').to_string();
        }
        if let Ok(raw) = std::fs::read(format!("{dir}{}", slashes(&inc))) {
            let text = String::from_utf8_lossy(&raw).into_owned();
            for l in text.lines() {
                let l = trim(l);
                if !l.is_empty() && !l.starts_with(';') {
                    parse_alias_line(l, dir, p);
                }
            }
        }
        return;
    }
    if let Some(eq) = line.find('=') {
        let mut file = trim(&line[eq + 1..]).to_string();
        if let Some(semi) = file.find(';') {
            file = trim(&file[..semi]).to_string();
        }
        p.aliases.push((trim(&line[..eq]).to_string(), slashes(&file)));
    }
}

fn parse_map_line(line: &str, dir: &str, p: &mut Project) {
    if let Some(rest) = line.strip_prefix("#define") {
        let rest = trim(rest);
        if let Some(sp) = rest.find([' ', '\t']) {
            let name = trim(&rest[..sp]).to_string();
            let v = trim(&rest[sp..]);
            let id = if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
                u32::from_str_radix(h, 16).unwrap_or(0)
            } else {
                v.parse().unwrap_or(0)
            };
            p.map_defs.push((name, id));
        }
    } else if let Some(rest) = line.strip_prefix("#include") {
        let mut inc = trim(rest).to_string();
        if inc.starts_with('"') || inc.starts_with('<') {
            inc = inc[1..inc.len() - 1].to_string();
        }
        if let Ok(raw) = std::fs::read(format!("{dir}{}", slashes(&inc))) {
            let text = String::from_utf8_lossy(&raw).into_owned();
            for l in text.lines() {
                let l = trim(l);
                if !l.is_empty() {
                    parse_map_line(l, dir, p);
                }
            }
        }
    }
}

fn parse_hhp(path: &str) -> std::io::Result<Project> {
    let raw = std::fs::read(path)?;
    let mut text = String::from_utf8_lossy(&raw).into_owned();
    if let Some(s) = text.strip_prefix('\u{feff}') {
        text = s.to_string();
    }
    let dirp = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = if dirp.is_empty() { String::new() } else { slashes(&dirp) + "/" };

    let mut p = Project { dir: dir.clone(), ..Default::default() };
    let mut section = String::new();
    let mut seen: HashSet<String> = HashSet::new();
    for line in text.lines() {
        let line = trim(line);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = lower(&line[1..line.len() - 1]);
            continue;
        }
        match section.as_str() {
            "options" => {
                if let Some(eq) = line.find('=') {
                    p.options.insert(lower(trim(&line[..eq])), trim(&line[eq + 1..]).to_string());
                }
            }
            "files" => {
                let f = slashes(line);
                if seen.insert(lower(&f)) {
                    p.files.push(f);
                }
            }
            "windows" => p.window_lines.push(line.to_string()),
            "alias" => parse_alias_line(line, &dir, &mut p),
            "map" => parse_map_line(line, &dir, &mut p),
            "merge files" => p.merge_files.push(slashes(line)),
            "subsets" => p.subsets.push(line.to_string()),
            "infotypes" => p.info_types.push(line.to_string()),
            _ => {}
        }
    }
    Ok(p)
}

fn parse_lcid(language: &str) -> u32 {
    let tok = language.split_whitespace().next().unwrap_or("");
    let v = if let Some(h) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        tok.parse::<u32>().ok()
    };
    v.filter(|&x| x != 0).unwrap_or(0x409)
}

// ---------------- [WINDOWS] ----------------

const WP_PROPERTIES: u32 = 0x0002;
const WP_STYLES: u32 = 0x0004;
const WP_EXSTYLES: u32 = 0x0008;
const WP_RECT: u32 = 0x0010;
const WP_NAV_WIDTH: u32 = 0x0020;
const WP_SHOWSTATE: u32 = 0x0040;
const WP_TB_FLAGS: u32 = 0x0100;
const WP_EXPANSION: u32 = 0x0200;
const WP_TABPOS: u32 = 0x0400;
const WP_CUR_TAB: u32 = 0x2000;

#[derive(Default)]
struct Window {
    typ: String,
    caption: String,
    toc: String,
    index: String,
    default_file: String,
    home: String,
    jump1_file: String,
    jump1_text: String,
    jump2_file: String,
    jump2_text: String,
    nav_style: u32,
    nav_width: u32,
    buttons: u32,
    rect: [i32; 4],
    styles: u32,
    ex_styles: u32,
    show_state: u32,
    nav_closed: u32,
    nav_default: u32,
    nav_pos: u32,
    notify_id: u32,
    valid_flags: u32,
}

fn parse_int(s: &str) -> u32 {
    let s = trim(s);
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).unwrap_or(0)
    } else {
        s.parse().unwrap_or(0)
    }
}

fn parse_window_line(raw_line: &str) -> Window {
    let mut line: Vec<u8> = raw_line.bytes().collect();
    if let Some(eq) = line.iter().position(|&c| c == b'=') {
        line[eq] = b',';
    }
    let s = String::from_utf8_lossy(&line).into_owned();
    let b = s.as_bytes();
    let mut tok: Vec<String> = Vec::new();
    let mut i = 0;
    while i <= b.len() {
        let mut cur = String::new();
        if i < b.len() && b[i] == b'"' {
            let close = b[i + 1..].iter().position(|&c| c == b'"').map(|x| i + 1 + x);
            match close {
                Some(c) => {
                    cur = s[i + 1..c].to_string();
                    i = c + 1;
                    i = s[i..].find(',').map(|x| i + x + 1).unwrap_or(b.len() + 1);
                }
                None => i = b.len() + 1,
            }
        } else {
            let comma = s[i..].find(',').map(|x| i + x).unwrap_or(b.len());
            cur = trim(&s[i..comma]).to_string();
            i = comma + 1;
        }
        tok.push(cur);
    }

    let mut w = Window::default();
    let str_at = |idx: usize| tok.get(idx).cloned().unwrap_or_default();
    let num = |idx: usize, bit: u32, w: &mut Window| -> u32 {
        if idx >= tok.len() || tok[idx].is_empty() {
            return 0;
        }
        if bit != 0 {
            w.valid_flags |= bit;
        }
        parse_int(&tok[idx])
    };

    w.typ = str_at(0);
    w.caption = str_at(1);
    w.toc = slashes(&str_at(2));
    w.index = slashes(&str_at(3));
    w.default_file = slashes(&str_at(4));
    w.home = slashes(&str_at(5));
    w.jump1_file = slashes(&str_at(6));
    w.jump1_text = str_at(7);
    w.jump2_file = slashes(&str_at(8));
    w.jump2_text = str_at(9);
    w.nav_style = num(10, WP_PROPERTIES, &mut w);
    w.nav_width = num(11, WP_NAV_WIDTH, &mut w);
    w.buttons = num(12, WP_TB_FLAGS, &mut w);

    let mut idx = 13;
    if idx < tok.len() && tok[idx].starts_with('[') {
        let mut any = false;
        let mut k = 0;
        while k < 4 && idx < tok.len() {
            let mut v = tok[idx].clone();
            v = v.replace('[', "");
            let last = v.contains(']');
            if let Some(br) = v.find(']') {
                v = v[..br].to_string();
            }
            if !trim(&v).is_empty() {
                any = true;
            }
            w.rect[k] = trim(&v).parse().unwrap_or(0);
            idx += 1;
            if last {
                break;
            }
            k += 1;
        }
        if any {
            w.valid_flags |= WP_RECT;
        }
    } else if idx < tok.len() {
        idx += 1;
    }
    w.styles = num(idx, WP_STYLES, &mut w);
    idx += 1;
    w.ex_styles = num(idx, WP_EXSTYLES, &mut w);
    idx += 1;
    w.show_state = num(idx, WP_SHOWSTATE, &mut w);
    idx += 1;
    w.nav_closed = num(idx, WP_EXPANSION, &mut w);
    idx += 1;
    w.nav_default = num(idx, WP_CUR_TAB, &mut w);
    idx += 1;
    w.nav_pos = num(idx, WP_TABPOS, &mut w);
    idx += 1;
    w.notify_id = num(idx, 0, &mut w);
    w
}

// ---------------- HTML scanning ----------------

fn is_html_name(name: &str) -> bool {
    let l = lower(name);
    l.contains(".ht") && !l.contains(".hhc") && !l.contains(".hhk")
}

fn extract_title_s(html: &[u8], cp: u32) -> Vec<u8> {
    let cps = decode_text(html, cp);
    let lc = |c: u32| if (b'A' as u32..=b'Z' as u32).contains(&c) { c + 32 } else { c };
    let find = |pat: &str, from: usize| -> Option<usize> {
        let pb: Vec<u32> = pat.bytes().map(|c| c as u32).collect();
        if pb.len() > cps.len() {
            return None;
        }
        (from..=cps.len() - pb.len()).find(|&i| (0..pb.len()).all(|j| lc(cps[i + j]) == pb[j]))
    };
    let mut t = match find("<title", 0) {
        Some(x) => x,
        None => return Vec::new(),
    };
    while t < cps.len() && cps[t] != b'>' as u32 {
        t += 1;
    }
    if t >= cps.len() {
        return Vec::new();
    }
    t += 1;
    let end = match find("</title", t) {
        Some(x) => x,
        None => return Vec::new(),
    };
    let mut title: Vec<u32> = Vec::new();
    let mut ws = false;
    for &c in &cps[t..end] {
        if c == b' ' as u32 || c == b'\t' as u32 || c == b'\r' as u32 || c == b'\n' as u32 {
            ws = !title.is_empty();
        } else {
            if ws {
                title.push(b' ' as u32);
            }
            ws = false;
            title.push(c);
        }
    }
    encode_codepage(&title, cp)
}

fn is_word_char_b(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

fn extract_refs(html: &[u8], out: &mut Vec<String>) {
    let text = html;
    let lc: Vec<u8> = text.iter().map(|&b| if b.is_ascii_uppercase() { b + 32 } else { b }).collect();
    for key in [b"href".as_slice(), b"src".as_slice()] {
        let mut pos = 0;
        while let Some(found) = lc[pos..].windows(key.len()).position(|w| w == key) {
            let at = pos + found;
            pos = at + key.len();
            if at > 0 && is_word_char_b(lc[at - 1]) {
                continue;
            }
            let mut i = pos;
            while i < text.len() && text[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= text.len() || text[i] != b'=' {
                continue;
            }
            i += 1;
            while i < text.len() && text[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value: Vec<u8> = Vec::new();
            if i < text.len() && (text[i] == b'"' || text[i] == b'\'') {
                let q = text[i];
                i += 1;
                while i < text.len() && text[i] != q {
                    value.push(text[i]);
                    i += 1;
                }
            } else {
                while i < text.len() && text[i] != b'>' && !text[i].is_ascii_whitespace() {
                    value.push(text[i]);
                    i += 1;
                }
            }
            if !value.is_empty() {
                out.push(String::from_utf8_lossy(&value).into_owned());
            }
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%'));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve_ref(base_dir: &str, link: &str) -> String {
    let link = trim(link);
    if link.is_empty() || link.starts_with('#') {
        return String::new();
    }
    if let Some(colon) = link.find(':') {
        if link.find('/').map(|s| s > colon).unwrap_or(true) {
            return String::new(); // scheme / drive
        }
    }
    let cut = link.find(['#', '?']).unwrap_or(link.len());
    let link = slashes(&percent_decode(&link[..cut]));
    if link.is_empty() {
        return String::new();
    }
    let full = if link.starts_with('/') {
        link[1..].to_string()
    } else {
        format!("{base_dir}{link}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in full.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if parts.pop().is_none() {
                return String::new();
            }
        } else {
            parts.push(seg);
        }
    }
    parts.join("/")
}

fn dir_of(rel: &str) -> String {
    match rel.rfind('/') {
        Some(s) => rel[..=s].to_string(),
        None => String::new(),
    }
}

// ---------------- #STRINGS / #URLSTR+#URLTBL / #TOPICS ----------------

/// Encodes a (possibly non-ASCII) string to raw codepage bytes for storage in the
/// byte-oriented metadata files (#STRINGS).
fn enc_cp(s: &str, cp: u32) -> Vec<u8> {
    encode_codepage(&decode_auto(s, cp), cp)
}

struct Strings {
    buf: Buf,
    map: HashMap<Vec<u8>, u32>,
}
impl Strings {
    fn new() -> Self {
        Strings { buf: Buf::new(), map: HashMap::new() }
    }
    fn add(&mut self, s: &[u8]) -> u32 {
        if self.buf.is_empty() {
            self.buf.u8(0);
        }
        if s.is_empty() {
            return 0;
        }
        if let Some(&o) = self.map.get(s) {
            return o;
        }
        let mut pos = self.buf.len();
        let next_block = (pos & !0xFFF) + 0x1000;
        if pos + s.len() + 1 > next_block && s.len() + 1 <= 0x1000 {
            self.buf.zeros(next_block - pos);
            pos = next_block;
        }
        self.buf.raw(s);
        self.buf.u8(0);
        self.map.insert(s.to_vec(), pos as u32);
        pos as u32
    }
    fn add_str(&mut self, s: &str, cp: u32) -> u32 {
        self.add(&enc_cp(s, cp))
    }
}

struct Urls {
    urlstr: Buf,
    urltbl: Buf,
    strmap: HashMap<String, u32>,
}
impl Urls {
    fn new() -> Self {
        Urls { urlstr: Buf::new(), urltbl: Buf::new(), strmap: HashMap::new() }
    }
    fn add_urlstr(&mut self, url: &str) -> u32 {
        if let Some(&o) = self.strmap.get(url) {
            return o;
        }
        let entry_len = 9 + url.len();
        let rem = 0x4000 - self.urlstr.len() % 0x4000;
        if rem < entry_len {
            self.urlstr.zeros(rem);
        }
        if self.urlstr.len() % 0x4000 == 0 {
            self.urlstr.u8(0);
        }
        let pos = self.urlstr.len() as u32;
        self.urlstr.u32(0);
        self.urlstr.u32(0);
        self.urlstr.strz(url);
        self.strmap.insert(url.to_string(), pos);
        pos
    }
    fn add_url(&mut self, url: &str, topic_index: u32) -> u32 {
        let us = self.add_urlstr(url);
        if self.urltbl.len() & 0xFFF == 0xFFC {
            self.urltbl.u32(0);
        }
        let pos = self.urltbl.len() as u32;
        self.urltbl.u32(0);
        self.urltbl.u32(topic_index);
        self.urltbl.u32(us);
        pos
    }
}

struct Topics {
    buf: Buf,
    by_url: HashMap<String, u32>,
}
impl Topics {
    fn new() -> Self {
        Topics { buf: Buf::new(), by_url: HashMap::new() }
    }
    fn count(&self) -> u32 {
        (self.buf.len() / 16) as u32
    }
    fn add(&mut self, strings: &mut Strings, urls: &mut Urls, title: &[u8], url: &str, code: i32) -> u32 {
        let mut url = url.to_string();
        if url.starts_with('/') {
            url.remove(0);
        }
        let topic_index = self.count();
        let str_off = if title.is_empty() { 0xFFFFFFFFu32 } else { strings.add(title) };
        let tbl_off = urls.add_url(&url, topic_index);
        let in_contents: u16 = if code >= 0 {
            code as u16
        } else if url.contains('#') {
            0
        } else if title.is_empty() {
            2
        } else {
            6
        };
        self.buf.u32(0);
        self.buf.u32(str_off);
        self.buf.u32(tbl_off);
        self.buf.u16(in_contents);
        self.buf.u16(0);
        self.by_url.entry(lower(&url)).or_insert(topic_index);
        topic_index
    }
    fn find(&self, url: &str) -> i32 {
        let mut url = url.to_string();
        if url.starts_with('/') {
            url.remove(0);
        }
        self.by_url.get(&lower(&url)).map(|&v| v as i32).unwrap_or(-1)
    }
    fn patch_toc_offset(&mut self, topic: u32, value: u32) {
        let off = topic as usize * 16;
        self.buf.v[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn sys_entry_str(b: &mut Buf, code: u16, value: &str) {
    b.u16(code);
    b.u16(value.len() as u16 + 1);
    b.strz(value);
}

include!("builder_parts.rs");
