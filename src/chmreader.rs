//! CHM (ITSF) reader: directory listing and file extraction. Port of chmreader.cpp.

use std::io::Write;
use std::path::Path;

use crate::lzxdecode::lzx_decompress;

pub struct ChmEntry {
    pub name: String,
    pub section: u32,
    pub offset: u64,
    pub length: u64,
}

pub struct ChmFile {
    file: Vec<u8>,
    entries: Vec<ChmEntry>,
    content_offset: u64,
    section1: Option<Vec<u8>>,
}

fn rd32(v: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([v[p], v[p + 1], v[p + 2], v[p + 3]])
}
fn rd64(v: &[u8], p: usize) -> u64 {
    rd32(v, p) as u64 | ((rd32(v, p + 4) as u64) << 32)
}
fn encint(v: &[u8], p: &mut usize) -> u64 {
    let mut x = 0u64;
    loop {
        let b = v[*p];
        *p += 1;
        x = (x << 7) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            break;
        }
    }
    x
}
fn window_bits_from_bytes(bytes: u32) -> u32 {
    let mut b = 0;
    while (1u32 << b) < bytes {
        b += 1;
    }
    b
}

impl ChmFile {
    pub fn open(path: &str) -> Result<ChmFile, String> {
        let file = std::fs::read(path).map_err(|e| format!("cannot open {path}: {e}"))?;
        if file.len() < 0x60 || &file[..4] != b"ITSF" {
            return Err("not an ITSF (CHM) file".into());
        }
        let hs1 = rd64(&file, 0x48) as usize;
        let content_offset = rd64(&file, 0x58);
        if hs1 + 0x54 > file.len() || &file[hs1..hs1 + 4] != b"ITSP" {
            return Err("bad ITSP directory".into());
        }
        let chunk_size = rd32(&file, hs1 + 0x10) as usize;
        let n_chunks = rd32(&file, hs1 + 0x2C) as usize;
        let dir_base = hs1 + 0x54;
        let mut entries = Vec::new();
        for c in 0..n_chunks {
            let base = dir_base + c * chunk_size;
            if base + chunk_size > file.len() || &file[base..base + 4] != b"PMGL" {
                continue;
            }
            let free = rd32(&file, base + 4) as usize;
            let mut p = base + 0x14;
            let end = base + chunk_size - free;
            while p < end {
                let nlen = encint(&file, &mut p) as usize;
                let name = String::from_utf8_lossy(&file[p..p + nlen]).into_owned();
                p += nlen;
                let section = encint(&file, &mut p) as u32;
                let offset = encint(&file, &mut p);
                let length = encint(&file, &mut p);
                entries.push(ChmEntry { name, section, offset, length });
            }
        }
        Ok(ChmFile { file, entries, content_offset, section1: None })
    }

    pub fn entries(&self) -> &[ChmEntry] {
        &self.entries
    }

    fn find(&self, name: &str) -> Option<&ChmEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    fn ensure_section1(&mut self) -> Result<(), String> {
        if self.section1.is_some() {
            return Ok(());
        }
        let content = self
            .find("::DataSpace/Storage/MSCompressed/Content")
            .ok_or("missing Content")?;
        let control = self
            .find("::DataSpace/Storage/MSCompressed/ControlData")
            .ok_or("missing ControlData")?;
        let span = self
            .find("::DataSpace/Storage/MSCompressed/SpanInfo")
            .ok_or("missing SpanInfo")?;
        let (c_off, c_len) = (content.offset, content.length);
        let ctl = (self.content_offset + control.offset) as usize;
        if &self.file[ctl + 4..ctl + 8] != b"LZXC" {
            return Err("section is not LZX-compressed".into());
        }
        let version = rd32(&self.file, ctl + 8);
        let mut reset_iv = rd32(&self.file, ctl + 12);
        let mut window = rd32(&self.file, ctl + 16);
        if version == 2 {
            reset_iv *= 0x8000;
            window *= 0x8000;
        }
        let uncompressed = rd64(&self.file, (self.content_offset + span.offset) as usize);
        let cstart = (self.content_offset + c_off) as usize;
        let data = lzx_decompress(
            &self.file[cstart..cstart + c_len as usize],
            uncompressed,
            reset_iv,
            window_bits_from_bytes(window),
        )?;
        self.section1 = Some(data);
        Ok(())
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let e = self.find(name).ok_or_else(|| format!("no such entry: {name}"))?;
        let (section, offset, length) = (e.section, e.offset, e.length);
        if section == 0 {
            let start = (self.content_offset + offset) as usize;
            if start + length as usize > self.file.len() {
                return Err(format!("entry out of range: {name}"));
            }
            return Ok(self.file[start..start + length as usize].to_vec());
        }
        self.ensure_section1()?;
        let s1 = self.section1.as_ref().unwrap();
        if (offset + length) as usize > s1.len() {
            return Err(format!("entry out of range in section: {name}"));
        }
        Ok(s1[offset as usize..(offset + length) as usize].to_vec())
    }
}

pub fn chm_list(path: &str) -> i32 {
    match ChmFile::open(path) {
        Ok(chm) => {
            for e in chm.entries() {
                println!(
                    "{:<8} {:>10}  {}",
                    if e.section != 0 { "lzx" } else { "raw" },
                    e.length,
                    e.name
                );
            }
            0
        }
        Err(e) => {
            eprintln!("rustchm: {e}");
            1
        }
    }
}

pub fn chm_extract(path: &str, out_dir: &str) -> i32 {
    let mut chm = match ChmFile::open(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rustchm: {e}");
            return 1;
        }
    };
    let names: Vec<(String, u64)> = chm
        .entries()
        .iter()
        .filter(|e| e.name.starts_with('/') && e.length > 0)
        .map(|e| (e.name.clone(), e.length))
        .collect();
    let mut count = 0;
    let mut failures = 0;
    for (name, _) in names {
        match chm.read(&name) {
            Ok(data) => {
                let dst = Path::new(out_dir).join(&name[1..]); // strip leading '/'
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::File::create(&dst).and_then(|mut f| f.write_all(&data)) {
                    Ok(_) => count += 1,
                    Err(e) => {
                        eprintln!("rustchm: cannot write {}: {e}", dst.display());
                        failures += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("rustchm: {e}");
                failures += 1;
            }
        }
    }
    println!("extracted {count} files to {out_dir}");
    if failures > 0 {
        1
    } else {
        0
    }
}
