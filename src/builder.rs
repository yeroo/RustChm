//! Compiles an HTML Help project (.hhp) into a .chm. Minimal core port of
//! FastChm's builder.cpp (content + #SYSTEM/#STRINGS/#TOPICS/#URLSTR/#URLTBL +
//! ::DataSpace + LZX). Sitemap/FTS/binary-index features come later.

use std::collections::HashMap;
use std::path::Path;

use crate::bytebuf::Buf;
use crate::chmwriter::{write_container, DirEntry};
use crate::lzx::lzx_compress;

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

struct Project {
    dir: String,
    options: HashMap<String, String>,
    files: Vec<String>,
}

impl Project {
    fn opt(&self, k: &str) -> String {
        self.options.get(k).cloned().unwrap_or_default()
    }
}

fn parse_hhp(path: &str) -> std::io::Result<Project> {
    let raw = std::fs::read(path)?;
    let mut text = String::from_utf8_lossy(&raw).into_owned();
    if let Some(stripped) = text.strip_prefix('\u{feff}') {
        text = stripped.to_string();
    }
    let dirp = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = if dirp.is_empty() { String::new() } else { slashes(&dirp) + "/" };

    let mut p = Project { dir, options: HashMap::new(), files: Vec::new() };
    let mut section = String::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = lower(&line[1..line.len() - 1]);
            continue;
        }
        if section == "options" {
            if let Some(eq) = line.find('=') {
                p.options.insert(lower(line[..eq].trim()), line[eq + 1..].trim().to_string());
            }
        } else if section == "files" {
            let f = slashes(line);
            if seen.insert(lower(&f)) {
                p.files.push(f);
            }
        }
    }
    Ok(p)
}

fn parse_lcid(language: &str) -> u32 {
    // accepts "0x409 ..." or decimal
    let tok = language.split_whitespace().next().unwrap_or("");
    let v = if let Some(h) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        tok.parse::<u32>().ok()
    };
    v.filter(|&x| x != 0).unwrap_or(0x409)
}

fn extract_title(html: &[u8]) -> String {
    let text = String::from_utf8_lossy(html);
    let lc = text.to_ascii_lowercase();
    let t = match lc.find("<title") {
        Some(x) => x,
        None => return String::new(),
    };
    let gt = match lc[t..].find('>') {
        Some(x) => t + x + 1,
        None => return String::new(),
    };
    let end = match lc[gt..].find("</title") {
        Some(x) => gt + x,
        None => return String::new(),
    };
    let raw = &text[gt..end];
    let mut out = String::new();
    let mut ws = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            ws = !out.is_empty();
        } else {
            if ws {
                out.push(' ');
            }
            ws = false;
            out.push(c);
        }
    }
    out
}

// ---- #STRINGS ----
struct Strings {
    buf: Buf,
    map: HashMap<String, u32>,
}
impl Strings {
    fn new() -> Self {
        Strings { buf: Buf::new(), map: HashMap::new() }
    }
    fn add(&mut self, s: &str) -> u32 {
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
        self.buf.strz(s);
        self.map.insert(s.to_string(), pos as u32);
        pos as u32
    }
}

// ---- #URLSTR / #URLTBL ----
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
}
impl Topics {
    fn new() -> Self {
        Topics { buf: Buf::new() }
    }
    fn add(&mut self, strings: &mut Strings, urls: &mut Urls, title: &str, mut url: String, code: i32) {
        if url.starts_with('/') {
            url.remove(0);
        }
        let topic_index = (self.buf.len() / 16) as u32;
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
    }
}

fn sys_entry_str(b: &mut Buf, code: u16, value: &str) {
    b.u16(code);
    b.u16(value.len() as u16 + 1);
    b.strz(value);
}

fn build_namelist() -> Vec<u8> {
    let mut b = Buf::new();
    b.u16(0);
    b.u16(2);
    for name in ["Uncompressed", "MSCompressed"] {
        b.u16(name.len() as u16);
        for c in name.bytes() {
            b.u16(c as u16);
        }
        b.u16(0);
    }
    let words = (b.len() / 2) as u16;
    b.v[0] = words as u8;
    b.v[1] = (words >> 8) as u8;
    b.v
}

fn build_control_data() -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(6);
    b.raw(b"LZXC");
    b.u32(2); // version
    b.u32(2); // reset interval (0x8000 units)
    b.u32(2); // window size (0x8000 units)
    b.u32(1); // cache size
    b.u32(0);
    b.u32(0);
    b.v
}

fn build_reset_table(uncompressed: u64, compressed: u64, frame_starts: &[u64]) -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(2);
    b.u32(frame_starts.len() as u32);
    b.u32(8);
    b.u32(0x28);
    b.u64(uncompressed);
    b.u64(compressed);
    b.u64(0x8000);
    for &off in frame_starts {
        b.u64(off);
    }
    b.v
}

fn build_transform_list() -> Vec<u8> {
    let g = b"{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}";
    let mut b = Buf::new();
    for i in 0..19 {
        b.u16(g[i] as u16);
    }
    b.v
}

fn build_system(p: &Project, lcid: u32, hhc: &str, hhk: &str) -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(3);
    b.u16(10);
    b.u16(4);
    b.u32(0); // timestamp (deterministic)
    sys_entry_str(&mut b, 9, concat!("rustchm ", env!("CARGO_PKG_VERSION")));
    b.u16(4);
    b.u16(36);
    b.u32(lcid);
    b.u32(0);
    b.u32(0);
    b.u32(0);
    b.u32(0);
    b.u64(0);
    b.u32(0);
    b.u32(0);
    let def_topic = slashes(&p.opt("default topic"));
    if !def_topic.is_empty() {
        sys_entry_str(&mut b, 2, &def_topic);
    }
    if !p.opt("title").is_empty() {
        sys_entry_str(&mut b, 3, &p.opt("title"));
    }
    if !p.opt("default font").is_empty() {
        sys_entry_str(&mut b, 16, &p.opt("default font"));
    }
    if !hhc.is_empty() {
        sys_entry_str(&mut b, 0, hhc);
    }
    if !hhk.is_empty() {
        sys_entry_str(&mut b, 1, hhk);
    }
    b.v
}

pub fn compile_project(hhp_path: &str, out_override: &str) -> Result<(Stats, String), String> {
    let mut p = parse_hhp(hhp_path).map_err(|e| format!("cannot read project: {e}"))?;
    let lcid = parse_lcid(&p.opt("language"));
    let hhc = slashes(&p.opt("contents file"));
    let hhk = slashes(&p.opt("index file"));

    let ensure = |files: &mut Vec<String>, f: &str| {
        if f.is_empty() {
            return;
        }
        if !files.iter().any(|e| lower(e) == lower(f)) {
            files.push(f.to_string());
        }
    };
    ensure(&mut p.files, &hhc);
    ensure(&mut p.files, &hhk);
    if p.files.is_empty() {
        return Err("project has no [FILES]".into());
    }

    let mut strings = Strings::new();
    let mut urls = Urls::new();
    let mut topics = Topics::new();
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut section1 = Buf::new();

    let mut add_sec1 = |entries: &mut Vec<DirEntry>, section1: &mut Buf, name: String, data: &[u8]| {
        entries.push(DirEntry { name, section: 1, offset: section1.len() as u64, size: data.len() as u64 });
        section1.raw(data);
    };

    let mut file_count = 0;
    for f in &p.files {
        let path = format!("{}{}", p.dir, f);
        let data = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
        add_sec1(&mut entries, &mut section1, format!("/{f}"), &data);
        let lf = lower(f);
        if lf.contains(".ht") && !lf.contains(".hhc") && !lf.contains(".hhk") {
            topics.add(&mut strings, &mut urls, &extract_title(&data), format!("/{f}"), -1);
        }
        file_count += 1;
    }
    if !hhc.is_empty() {
        topics.add(&mut strings, &mut urls, "", hhc.clone(), 2);
    }
    if !hhk.is_empty() {
        topics.add(&mut strings, &mut urls, "", hhk.clone(), 2);
    }

    if !topics.buf.is_empty() {
        add_sec1(&mut entries, &mut section1, "/#TOPICS".into(), &topics.buf.v);
    }
    if !urls.urlstr.is_empty() {
        add_sec1(&mut entries, &mut section1, "/#URLSTR".into(), &urls.urlstr.v);
    }
    if !urls.urltbl.is_empty() {
        add_sec1(&mut entries, &mut section1, "/#URLTBL".into(), &urls.urltbl.v);
    }
    if strings.buf.is_empty() {
        strings.buf.u8(0);
    }
    add_sec1(&mut entries, &mut section1, "/#STRINGS".into(), &strings.buf.v);

    let uncompressed = section1.len() as u64;
    let lzx = lzx_compress(&section1.v);

    // ---- section 0 ----
    let mut section0 = Buf::new();
    let mut add_sec0 = |entries: &mut Vec<DirEntry>, section0: &mut Buf, name: String, data: &[u8]| {
        entries.push(DirEntry { name, section: 0, offset: section0.len() as u64, size: data.len() as u64 });
        section0.raw(data);
    };
    entries.push(DirEntry { name: "/#ITBITS".into(), section: 0, offset: 0, size: 0 });
    add_sec0(&mut entries, &mut section0, "/#SYSTEM".into(), &build_system(&p, lcid, &hhc, &hhk));
    add_sec0(&mut entries, &mut section0, "::DataSpace/NameList".into(), &build_namelist());
    add_sec0(&mut entries, &mut section0, "::DataSpace/Storage/MSCompressed/ControlData".into(), &build_control_data());
    {
        let mut span = Buf::new();
        span.u64(uncompressed);
        add_sec0(&mut entries, &mut section0, "::DataSpace/Storage/MSCompressed/SpanInfo".into(), &span.v);
    }
    add_sec0(&mut entries, &mut section0, "::DataSpace/Storage/MSCompressed/Transform/List".into(), &build_transform_list());
    add_sec0(
        &mut entries,
        &mut section0,
        "::DataSpace/Storage/MSCompressed/Transform/{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}/InstanceData/ResetTable".into(),
        &build_reset_table(uncompressed, lzx.data.len() as u64, &lzx.frame_starts),
    );
    entries.push(DirEntry {
        name: "::DataSpace/Storage/MSCompressed/Content".into(),
        section: 0,
        offset: section0.len() as u64,
        size: lzx.data.len() as u64,
    });

    // ---- output path ----
    let out = if !out_override.is_empty() {
        out_override.to_string()
    } else {
        let compiled = slashes(&p.opt("compiled file"));
        if !compiled.is_empty() {
            format!("{}{}", p.dir, compiled)
        } else {
            let stem = Path::new(hhp_path).file_stem().unwrap().to_string_lossy();
            format!("{}{}.chm", p.dir, stem)
        }
    };

    let size = write_container(&out, lcid, entries, &section0.v, &lzx.data)
        .map_err(|e| format!("write failed: {e}"))?;

    Ok((
        Stats { file_count, uncompressed, compressed: lzx.data.len() as u64, output: size },
        out,
    ))
}
