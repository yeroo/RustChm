//! Text encoding: decode source bytes (UTF-8/UTF-16/codepage) to Unicode code
//! points, and re-encode metadata as UTF-16LE or a single-byte codepage.
//! Port of FastChm's textenc.cpp.

use crate::codepage_tables::{CodepageTable, CODEPAGES};

fn find_cp(cp: u32) -> Option<&'static CodepageTable> {
    CODEPAGES.iter().find(|t| t.cp == cp)
}

fn decode_byte(b: u8, t: Option<&CodepageTable>) -> u32 {
    if b < 0x80 {
        b as u32
    } else if let Some(t) = t {
        t.high[(b - 0x80) as usize] as u32
    } else {
        b as u32
    }
}

/// Reads a UTF-8 sequence at `p`; returns code point and advances `p`. Invalid
/// bytes pass through as Latin-1 so nothing is lost.
fn next_utf8(d: &[u8], p: &mut usize) -> u32 {
    let b = d[*p];
    if b < 0x80 {
        *p += 1;
        return b as u32;
    }
    let len = if b >= 0xF0 {
        4
    } else if b >= 0xE0 {
        3
    } else if b >= 0xC0 {
        2
    } else {
        0
    };
    if len == 0 || *p + len > d.len() {
        *p += 1;
        return b as u32;
    }
    for i in 1..len {
        if d[*p + i] & 0xC0 != 0x80 {
            *p += 1;
            return b as u32;
        }
    }
    let mut cp = (b & (0x7F >> len)) as u32;
    for i in 1..len {
        cp = (cp << 6) | (d[*p + i] & 0x3F) as u32;
    }
    *p += len;
    cp
}

fn sniff_charset(d: &[u8]) -> u32 {
    let lim = d.len().min(2048);
    let s: String = d[..lim].iter().map(|&b| (b as char).to_ascii_lowercase()).collect();
    let pos = match s.find("charset") {
        Some(x) => x,
        None => return 0,
    };
    let mut i = pos + 7;
    let bytes = s.as_bytes();
    while i < bytes.len() && matches!(bytes[i], b'=' | b' ' | b'"' | b'\'' | b':') {
        i += 1;
    }
    let mut name = String::new();
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        name.push(bytes[i] as char);
        i += 1;
    }
    match name.as_str() {
        "utf-8" | "utf8" => 65001,
        "utf-16" | "utf-16le" | "unicode" => 1200,
        "iso-8859-1" | "latin1" => 1252,
        n if n.starts_with("windows-") => n[8..].parse().unwrap_or(0),
        _ => 0,
    }
}

pub fn codepage_for_lcid(lcid: u32) -> u32 {
    let primary = lcid & 0x3FF;
    let map = |l: u32| -> Option<u32> {
        Some(match l {
            0x0409 | 0x0809 | 0x0407 | 0x040C | 0x0410 | 0x040A | 0x0413 | 0x041D => 1252,
            0x0419 | 0x0422 | 0x0402 => 1251,
            0x041A | 0x0405 | 0x040E | 0x0415 | 0x0418 => 1250,
            0x0408 => 1253,
            0x041F => 1254,
            0x0427 | 0x0425 | 0x0426 => 1257,
            0x040D => 1255,
            0x0401 => 1256,
            0x041E => 874,
            0x0411 => 932,
            0x0804 => 936,
            0x0412 => 949,
            0x0404 => 950,
            0x042A => 1258,
            _ => return None,
        })
    };
    if let Some(c) = map(lcid) {
        return c;
    }
    match primary {
        0x19 | 0x22 | 0x02 => 1251,
        0x1A | 0x05 | 0x0E | 0x15 | 0x18 => 1250,
        0x08 => 1253,
        0x1F => 1254,
        0x27 | 0x25 | 0x26 => 1257,
        0x0D => 1255,
        0x01 => 1256,
        0x1E => 874,
        0x11 => 932,
        0x12 => 949,
        0x04 => 936,
        0x2A => 1258,
        _ => 1252,
    }
}

pub fn is_dbcs_codepage(cp: u32) -> bool {
    matches!(cp, 932 | 936 | 949 | 950 | 1361)
}

pub fn decode_codepage(s: &str, cp: u32) -> Vec<u32> {
    let t = find_cp(cp);
    s.bytes().map(|c| decode_byte(c, t)).collect()
}

/// valid multibyte UTF-8 → UTF-8, else `cp`.
pub fn decode_auto(s: &str, cp: u32) -> Vec<u32> {
    let bytes = s.as_bytes();
    let mut multibyte = false;
    let mut valid = true;
    let mut i = 0;
    while i < bytes.len() && valid {
        let b = bytes[i];
        if b < 0x80 {
            i += 1;
            continue;
        }
        let len = if b >= 0xF0 {
            4
        } else if b >= 0xE0 {
            3
        } else if b >= 0xC0 {
            2
        } else {
            0
        };
        if len == 0 || i + len > bytes.len() {
            valid = false;
            break;
        }
        for k in 1..len {
            if bytes[i + k] & 0xC0 != 0x80 {
                valid = false;
            }
        }
        multibyte = true;
        i += len;
    }
    if valid && multibyte {
        let mut out = Vec::new();
        let mut p = 0;
        while p < bytes.len() {
            out.push(next_utf8(bytes, &mut p));
        }
        out
    } else {
        decode_codepage(s, cp)
    }
}

pub fn decode_text(data: &[u8], fallback_cp: u32) -> Vec<u32> {
    if data.len() >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF {
        let mut out = Vec::new();
        let mut p = 3;
        while p < data.len() {
            out.push(next_utf8(data, &mut p));
        }
        return out;
    }
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
        return data[2..].chunks(2).filter(|c| c.len() == 2).map(|c| c[0] as u32 | ((c[1] as u32) << 8)).collect();
    }
    if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
        return data[2..].chunks(2).filter(|c| c.len() == 2).map(|c| ((c[0] as u32) << 8) | c[1] as u32).collect();
    }
    let mut cp = sniff_charset(data);
    if cp == 0 {
        cp = fallback_cp;
    }
    if cp == 65001 {
        let mut out = Vec::new();
        let mut p = 0;
        while p < data.len() {
            out.push(next_utf8(data, &mut p));
        }
        return out;
    }
    let t = find_cp(cp);
    data.iter().map(|&b| decode_byte(b, t)).collect()
}

pub fn append_utf16le(out: &mut Vec<u8>, cps: &[u32]) {
    for &c in cps {
        if c <= 0xFFFF {
            out.push(c as u8);
            out.push((c >> 8) as u8);
        } else {
            let c = c - 0x10000;
            let hi = 0xD800 + (c >> 10);
            let lo = 0xDC00 + (c & 0x3FF);
            out.push(hi as u8);
            out.push((hi >> 8) as u8);
            out.push(lo as u8);
            out.push((lo >> 8) as u8);
        }
    }
}

/// Encodes code points to raw bytes in a single-byte codepage (un-representable
/// points become '?'). DBCS / UTF-8 codepages fall back to UTF-8 bytes. Returns raw
/// bytes (not a String): these are metadata bytes stored verbatim in #STRINGS, and
/// are not necessarily valid UTF-8.
pub fn encode_codepage(cps: &[u32], cp: u32) -> Vec<u8> {
    if is_dbcs_codepage(cp) || cp == 65001 {
        let mut out: Vec<u8> = Vec::new();
        for &c in cps {
            if let Some(ch) = char::from_u32(c) {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
        return out;
    }
    let t = find_cp(cp);
    let mut out: Vec<u8> = Vec::new();
    for &c in cps {
        if c < 0x80 {
            out.push(c as u8);
            continue;
        }
        let mut mapped = b'?';
        if let Some(t) = t {
            for i in 0..128 {
                if t.high[i] as u32 == c {
                    mapped = 0x80 + i as u8;
                    break;
                }
            }
        } else if c <= 0xFF {
            mapped = c as u8;
        }
        out.push(mapped);
    }
    out
}
