//! ITSF/ITSP container writer. Port of FastChm's chmwriter.cpp.

use crate::bytebuf::{encint, Buf};
use std::io::Write;

#[derive(Clone)]
pub struct DirEntry {
    pub name: String, // full archive path, e.g. "/index.htm" or "::DataSpace/NameList"
    pub section: u32, // 0 = Uncompressed, 1 = MSCompressed
    pub offset: u64,
    pub size: u64,
}

const CHUNK: usize = 0x1000;
const PMGL_HDR: usize = 0x14;
const PMGI_HDR: usize = 0x08;
const QR_STRIDE: usize = 5; // quickref density 2 -> every 1+(1<<2) entries

fn lower(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        c + 32
    } else {
        c
    }
}

fn name_less(a: &str, b: &str) -> bool {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let n = ab.len().min(bb.len());
    for i in 0..n {
        let (ca, cb) = (lower(ab[i]), lower(bb[i]));
        if ca != cb {
            return ca < cb;
        }
    }
    ab.len() < bb.len()
}

struct ChunkBuilder {
    header_size: usize,
    body: Vec<u8>,
    entry_offsets: Vec<u16>,
    first_name: String,
}

impl ChunkBuilder {
    fn new(hdr: usize) -> Self {
        ChunkBuilder {
            header_size: hdr,
            body: Vec::new(),
            entry_offsets: Vec::new(),
            first_name: String::new(),
        }
    }
    fn quickref_bytes(count: usize) -> usize {
        if count == 0 {
            2
        } else {
            2 + 2 * ((count - 1) / QR_STRIDE)
        }
    }
    fn can_add(&self, entry_size: usize) -> bool {
        self.header_size + self.body.len() + entry_size + Self::quickref_bytes(self.entry_offsets.len() + 1)
            <= CHUNK
    }
    fn add(&mut self, name: &str, entry: &[u8]) {
        if self.entry_offsets.is_empty() {
            self.first_name = name.to_string();
        }
        self.entry_offsets.push(self.body.len() as u16);
        self.body.extend_from_slice(entry);
    }
    fn finalize(&self, chunk: &mut [u8]) {
        chunk[self.header_size..self.header_size + self.body.len()].copy_from_slice(&self.body);
        let count = self.entry_offsets.len();
        chunk[CHUNK - 2] = count as u8;
        chunk[CHUNK - 1] = (count >> 8) as u8;
        let mut j = 1;
        while j * QR_STRIDE < count {
            let off = self.entry_offsets[j * QR_STRIDE];
            chunk[CHUNK - 2 - 2 * j] = off as u8;
            chunk[CHUNK - 1 - 2 * j] = (off >> 8) as u8;
            j += 1;
        }
    }
    fn free_space(&self) -> u32 {
        (CHUNK - self.header_size - self.body.len()) as u32
    }
}

const G1D: [u8; 8] = [0x9E, 0x0C, 0x00, 0xA0, 0xC9, 0x22, 0xE6, 0xEC];
const G3D: [u8; 8] = [0x9D, 0xF9, 0x00, 0xA0, 0xC9, 0x22, 0xE6, 0xEC];

/// Writes a complete .chm. Returns Err on I/O failure.
pub fn write_container(
    path: &str,
    lang_id: u32,
    mut entries: Vec<DirEntry>,
    section0: &[u8],
    compressed: &[u8],
) -> std::io::Result<u64> {
    entries.sort_by(|a, b| {
        if name_less(&a.name, &b.name) {
            std::cmp::Ordering::Less
        } else if name_less(&b.name, &a.name) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });

    // ---- PMGL listing chunks ----
    let mut list_chunks: Vec<ChunkBuilder> = Vec::new();
    let mut cur = ChunkBuilder::new(PMGL_HDR);
    for e in &entries {
        let mut enc = Vec::new();
        encint(&mut enc, e.name.len() as u64);
        enc.extend_from_slice(e.name.as_bytes());
        encint(&mut enc, e.section as u64);
        encint(&mut enc, e.offset);
        encint(&mut enc, e.size);
        if !cur.can_add(enc.len()) {
            list_chunks.push(std::mem::replace(&mut cur, ChunkBuilder::new(PMGL_HDR)));
        }
        cur.add(&e.name, &enc);
    }
    if !cur.entry_offsets.is_empty() {
        list_chunks.push(cur);
    }
    let num_pmgl = list_chunks.len();

    // ---- PMGI index levels (only when more than one listing chunk) ----
    struct Ref {
        name: String,
        chunk: i32,
    }
    let mut level: Vec<Ref> = (0..num_pmgl)
        .map(|i| Ref { name: list_chunks[i].first_name.clone(), chunk: i as i32 })
        .collect();
    let mut index_chunks: Vec<ChunkBuilder> = Vec::new();
    let mut next_chunk_idx = num_pmgl as i32;
    let mut depth = 1u32;
    let mut root_chunk: i32 = -1;
    while level.len() > 1 {
        depth += 1;
        let mut next: Vec<Ref> = Vec::new();
        let mut ic = ChunkBuilder::new(PMGI_HDR);
        for r in &level {
            let mut enc = Vec::new();
            encint(&mut enc, r.name.len() as u64);
            enc.extend_from_slice(r.name.as_bytes());
            encint(&mut enc, r.chunk as u64);
            if !ic.can_add(enc.len()) {
                next.push(Ref { name: ic.first_name.clone(), chunk: next_chunk_idx });
                next_chunk_idx += 1;
                index_chunks.push(std::mem::replace(&mut ic, ChunkBuilder::new(PMGI_HDR)));
            }
            ic.add(&r.name, &enc);
        }
        if !ic.entry_offsets.is_empty() {
            next.push(Ref { name: ic.first_name.clone(), chunk: next_chunk_idx });
            next_chunk_idx += 1;
            index_chunks.push(ic);
        }
        level = next;
        root_chunk = level.last().unwrap().chunk;
    }
    let total_chunks = next_chunk_idx;

    // ---- assemble directory chunk bytes ----
    let mut chunks = vec![0u8; total_chunks as usize * CHUNK];
    for i in 0..num_pmgl {
        let p = i * CHUNK;
        let mut h = Buf::new();
        h.raw(b"PMGL");
        h.u32(list_chunks[i].free_space());
        h.u32(0);
        h.i32(if i == 0 { -1 } else { i as i32 - 1 });
        h.i32(if i == num_pmgl - 1 { -1 } else { i as i32 + 1 });
        chunks[p..p + h.len()].copy_from_slice(&h.v);
        list_chunks[i].finalize(&mut chunks[p..p + CHUNK]);
    }
    for (i, ic) in index_chunks.iter().enumerate() {
        let p = (num_pmgl + i) * CHUNK;
        let mut h = Buf::new();
        h.raw(b"PMGI");
        h.u32(ic.free_space());
        chunks[p..p + h.len()].copy_from_slice(&h.v);
        ic.finalize(&mut chunks[p..p + CHUNK]);
    }

    // ---- headers ----
    let hs1_len = 0x54u64 + chunks.len() as u64;
    let content_offset = 0x78u64 + hs1_len;
    let file_size = content_offset + section0.len() as u64 + compressed.len() as u64;

    let mut out = Buf::new();
    // ITSF header
    out.raw(b"ITSF");
    out.u32(3);
    out.u32(0x60);
    out.u32(1);
    out.u32(0); // timestamp (deterministic)
    out.u32(lang_id);
    out.guid(0x7C01FD10, 0x7BAA, 0x11D0, &G1D);
    out.guid(0x7C01FD11, 0x7BAA, 0x11D0, &G1D);
    out.u64(0x60);
    out.u64(0x18);
    out.u64(0x78);
    out.u64(hs1_len);
    out.u64(content_offset);
    // header section 0
    out.u32(0x01FE);
    out.u32(0);
    out.u64(file_size);
    out.u32(0);
    out.u32(0);
    // header section 1: ITSP directory header
    out.raw(b"ITSP");
    out.u32(1);
    out.u32(0x54);
    out.u32(0x0A);
    out.u32(0x1000);
    out.u32(2);
    out.u32(depth);
    out.i32(root_chunk);
    out.u32(0); // first PMGL
    out.u32(num_pmgl as u32 - 1); // last PMGL
    out.i32(-1);
    out.u32(total_chunks as u32);
    out.u32(lang_id);
    out.guid(0x5D02926A, 0x212E, 0x11D0, &G3D);
    out.u32(0x54);
    out.i32(-1);
    out.i32(-1);
    out.i32(-1);

    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(&out.v)?;
    f.write_all(&chunks)?;
    f.write_all(section0)?;
    f.write_all(compressed)?;
    f.flush()?;
    Ok(file_size)
}
