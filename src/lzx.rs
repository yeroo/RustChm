//! LZX encoder with CHM parameters (64K window, 64K reset interval). Port of
//! FastChm's lzx.cpp: 16-bit little-endian bitstream, bits placed MSB-first;
//! verbatim/aligned blocks; pretree-delta-coded Huffman trees; full state reset
//! each reset interval; 16-bit realignment every 0x8000 output bytes.

const FRAME: usize = 0x8000;
const INTERVAL: usize = 0x10000;
const NUM_CHARS: usize = 256;
const NUM_SLOTS: usize = 32; // window 2^16
const MAIN_SYMS: usize = NUM_CHARS + NUM_SLOTS * 8; // 512
const LEN_SYMS: usize = 249;
const PRETREE_SYMS: usize = 20;
const ALIGNED_SYMS: usize = 8;
const MIN_MATCH: usize = 2;
const MAX_MATCH: usize = 257;
const MAX_DIST: u32 = 0xFFFD;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: i32 = 128;
const BLOCK_VERBATIM: u32 = 1;
const BLOCK_ALIGNED: u32 = 2;
const LZX_LAZY: bool = true;

pub struct LzxResult {
    pub data: Vec<u8>,
    pub frame_starts: Vec<u64>,
    pub padded_size: u64,
}

struct Slots {
    extra: [u8; NUM_SLOTS + 1],
    base: [u32; NUM_SLOTS + 1],
}

fn slot_tables() -> Slots {
    let mut s = Slots {
        extra: [0; NUM_SLOTS + 1],
        base: [0; NUM_SLOTS + 1],
    };
    let mut b: u32 = 0;
    for i in 0..=NUM_SLOTS {
        s.extra[i] = if i < 2 {
            0
        } else {
            std::cmp::min(17, ((i - 2) >> 1) as u8)
        };
        s.base[i] = b;
        b += 1u32 << s.extra[i];
    }
    s
}

fn slot_for(st: &Slots, formatted: u32) -> usize {
    let mut lo = 0usize;
    let mut hi = NUM_SLOTS - 1;
    while lo < hi {
        let mid = (lo + hi + 1) >> 1;
        if st.base[mid] <= formatted {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    acc: u64,
    n: i32,
}

impl<'a> BitWriter<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        BitWriter { out, acc: 0, n: 0 }
    }
    fn put(&mut self, nbits: i32, value: u32) {
        if nbits == 0 {
            return;
        }
        self.acc = (self.acc << nbits) | (value as u64 & ((1u64 << nbits) - 1));
        self.n += nbits;
        while self.n >= 16 {
            let w = (self.acc >> (self.n - 16)) as u16;
            self.out.push((w & 0xFF) as u8);
            self.out.push((w >> 8) as u8);
            self.n -= 16;
        }
    }
    fn align16(&mut self) {
        if self.n != 0 {
            self.put(16 - self.n, 0);
        }
    }
}

/// Huffman code lengths bounded by `max_len`. All-zero freq -> empty tree; a single
/// used symbol gets length 1 along with the lowest other symbol.
fn huff_lengths(freq_in: &[u32], n: usize, max_len: i32, out_len: &mut [u8]) {
    for x in out_len.iter_mut().take(n) {
        *x = 0;
    }
    let mut freq: Vec<u32> = freq_in[..n].to_vec();
    let mut syms: Vec<usize> = (0..n).filter(|&i| freq[i] != 0).collect();
    if syms.is_empty() {
        return;
    }
    if syms.len() == 1 {
        out_len[syms[0]] = 1;
        out_len[if syms[0] == 0 { 1 } else { 0 }] = 1;
        return;
    }
    let m = syms.len();
    let mut f = vec![0u64; 2 * m - 1];
    let mut parent = vec![-1i32; 2 * m - 1];
    loop {
        syms.sort_by(|&a, &b| {
            if freq[a] != freq[b] {
                freq[a].cmp(&freq[b])
            } else {
                a.cmp(&b)
            }
        });
        for i in 0..m {
            f[i] = freq[syms[i]] as u64;
        }
        for p in parent.iter_mut() {
            *p = -1;
        }
        // two-queue merge: leaves f[0..m) sorted; internal nodes form a nondecreasing queue
        let mut leaf = 0usize;
        let mut inode = m;
        let mut next = m;
        while next < 2 * m - 1 {
            let a = if leaf < m && (inode >= next || f[leaf] <= f[inode]) {
                let r = leaf;
                leaf += 1;
                r
            } else {
                let r = inode;
                inode += 1;
                r
            };
            let b = if leaf < m && (inode >= next || f[leaf] <= f[inode]) {
                let r = leaf;
                leaf += 1;
                r
            } else {
                let r = inode;
                inode += 1;
                r
            };
            f[next] = f[a] + f[b];
            parent[a] = next as i32;
            parent[b] = next as i32;
            next += 1;
        }
        let mut max_depth = 0i32;
        for i in 0..m {
            let mut d = 0i32;
            let mut p = parent[i];
            while p >= 0 {
                d += 1;
                p = parent[p as usize];
            }
            max_depth = max_depth.max(d);
        }
        if max_depth <= max_len {
            for i in 0..m {
                let mut d = 0i32;
                let mut p = parent[i];
                while p >= 0 {
                    d += 1;
                    p = parent[p as usize];
                }
                out_len[syms[i]] = d as u8;
            }
            return;
        }
        for &s in &syms {
            freq[s] = std::cmp::max(1, freq[s] >> 1);
        }
    }
}

/// Canonical code assignment (count[0]-inclusive like FastChm; `put` masks to `len`
/// bits so the emitted codes are RFC-canonical).
fn assign_codes(len: &[u8], n: usize, code: &mut [u16]) {
    let mut count = [0u32; 17];
    for &l in len.iter().take(n) {
        count[l as usize] += 1;
    }
    let mut next = [0u32; 17];
    let mut c = 0u32;
    for l in 1..=16 {
        c = (c + count[l - 1]) << 1;
        next[l] = c;
    }
    for i in 0..n {
        code[i] = if len[i] != 0 {
            let v = next[len[i] as usize];
            next[len[i] as usize] += 1;
            v as u16
        } else {
            0
        };
    }
}

struct TreeItem {
    sym: u8,
    excess_bits: u8,
    excess: u8,
}

/// Pretree-encodes `n` code lengths (previous lengths all zero: one block per interval).
fn write_tree(bw: &mut BitWriter, lens: &[u8], n: usize) {
    let mut items: Vec<TreeItem> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let mut j = i;
        while j < n && lens[j] == lens[i] {
            j += 1;
        }
        let mut run = (j - i) as i32;
        let v = lens[i];
        if v == 0 {
            while run >= 20 {
                let e = std::cmp::min(run - 20, 31);
                items.push(TreeItem { sym: 18, excess_bits: 5, excess: e as u8 });
                run -= 20 + e;
            }
            while run >= 4 {
                let e = std::cmp::min(run - 4, 15);
                items.push(TreeItem { sym: 17, excess_bits: 4, excess: e as u8 });
                run -= 4 + e;
            }
            while run > 0 {
                items.push(TreeItem { sym: 0, excess_bits: 0, excess: 0 });
                run -= 1;
            }
        } else {
            let d = ((17 - v as i32) % 17) as u8;
            while run >= 4 {
                let e = if run >= 5 { 1 } else { 0 };
                items.push(TreeItem { sym: 19, excess_bits: 1, excess: e });
                items.push(TreeItem { sym: d, excess_bits: 0, excess: 0 });
                run -= 4 + e as i32;
            }
            while run > 0 {
                items.push(TreeItem { sym: d, excess_bits: 0, excess: 0 });
                run -= 1;
            }
        }
        i = j;
    }

    let mut freq = [0u32; PRETREE_SYMS];
    for it in &items {
        freq[it.sym as usize] += 1;
    }
    let mut plen = [0u8; PRETREE_SYMS];
    let mut pcode = [0u16; PRETREE_SYMS];
    huff_lengths(&freq, PRETREE_SYMS, 15, &mut plen);
    assign_codes(&plen, PRETREE_SYMS, &mut pcode);

    for s in 0..PRETREE_SYMS {
        bw.put(4, plen[s] as u32);
    }
    for it in &items {
        bw.put(plen[it.sym as usize] as i32, pcode[it.sym as usize] as u32);
        if it.excess_bits != 0 {
            bw.put(it.excess_bits as i32, it.excess as u32);
        }
    }
}

const TOK_MATCH: u32 = 0x8000_0000;

struct IntervalCoder {
    main_freq: Vec<u32>,
    len_freq: [u32; LEN_SYMS],
    aligned_freq: [u32; ALIGNED_SYMS],
    tokens: Vec<u32>,
}

impl IntervalCoder {
    fn new() -> Self {
        IntervalCoder {
            main_freq: vec![0; MAIN_SYMS],
            len_freq: [0; LEN_SYMS],
            aligned_freq: [0; ALIGNED_SYMS],
            tokens: Vec::new(),
        }
    }
    fn add_literal(&mut self, c: u8) {
        self.tokens.push(c as u32);
        self.main_freq[c as usize] += 1;
    }
    fn add_match(&mut self, st: &Slots, slot: usize, footer: u32, len: usize) {
        let token = TOK_MATCH
            | ((slot as u32) << 25)
            | (footer << 8)
            | ((len - MIN_MATCH) as u32);
        self.tokens.push(token);
        let len_header = std::cmp::min(len - MIN_MATCH, 7);
        self.main_freq[NUM_CHARS + (slot << 3 | len_header)] += 1;
        if len_header == 7 {
            self.len_freq[len - MIN_MATCH - 7] += 1;
        }
        if st.extra[slot] >= 3 {
            self.aligned_freq[(footer & 7) as usize] += 1;
        }
    }
}

fn match_len(a: &[u8], b: &[u8], max_len: usize) -> usize {
    let mut i = 0;
    while i < max_len && a[i] == b[i] {
        i += 1;
    }
    i
}

#[derive(Clone, Copy)]
struct Match {
    len: usize,
    dist: u32,
    rep: i32,
}

/// LZ parse of one reset interval [start,end) with one-step lazy matching.
fn parse_interval(
    st: &Slots,
    data: &[u8],
    start: usize,
    end: usize,
    coder: &mut IntervalCoder,
    head: &mut [i32],
    prev: &mut [i32],
) {
    for h in head.iter_mut() {
        *h = -1;
    }
    let mut r0: u32 = 1;
    let mut r1: u32 = 1;
    let mut r2: u32 = 1;

    let hash_at = |p: usize| -> usize {
        let h = ((data[p] as u32) << 16) | ((data[p + 1] as u32) << 8) | data[p + 2] as u32;
        ((h.wrapping_mul(2654435761)) >> (32 - HASH_BITS)) as usize
    };

    let find_best = |p: usize, r0: u32, r1: u32, r2: u32, head: &[i32], prev: &[i32]| -> Match {
        let mut m = Match { len: 0, dist: 0, rep: -1 };
        let frame_end = (p / FRAME + 1) * FRAME;
        let max_len = std::cmp::min(std::cmp::min(MAX_MATCH, end - p), frame_end - p);
        if max_len < MIN_MATCH {
            return m;
        }
        let reps = [r0, r1, r2];
        for r in 0..3 {
            if reps[r] as usize > p - start {
                continue;
            }
            let len = match_len(&data[p - reps[r] as usize..], &data[p..], max_len);
            if len >= (if r == 0 { 2 } else { 3 }) && len > m.len {
                m.len = len;
                m.dist = reps[r];
                m.rep = r as i32;
            }
        }
        if p + 2 < end {
            let mut chain = MAX_CHAIN;
            let mut cand = head[hash_at(p)];
            while cand >= 0 && chain > 0 {
                chain -= 1;
                let dist = (p - cand as usize) as u32;
                if dist > MAX_DIST {
                    break;
                }
                let candu = cand as usize;
                if !(m.len > 0
                    && (m.len >= max_len || data[candu + m.len] != data[p + m.len]))
                {
                    let len = match_len(&data[candu..], &data[p..], max_len);
                    if len > m.len && len >= 3 {
                        let fmt = dist + 2;
                        if !((fmt >= 64 && len < 4) || (fmt >= 2048 && len < 5)) {
                            m.len = len;
                            m.dist = dist;
                            m.rep = -1;
                        }
                    }
                }
                cand = prev[candu - start];
            }
        }
        m
    };

    let insert = |p: usize, head: &mut [i32], prev: &mut [i32]| {
        if p + 2 < end {
            let h = hash_at(p);
            prev[p - start] = head[h];
            head[h] = p as i32;
        }
    };

    let mut pos = start;
    while pos < end {
        let m = find_best(pos, r0, r1, r2, head, prev);
        if m.len >= 2 {
            insert(pos, head, prev);
            if LZX_LAZY
                && m.len < MAX_MATCH
                && pos + 1 < end
                && find_best(pos + 1, r0, r1, r2, head, prev).len > m.len
            {
                coder.add_literal(data[pos]);
                pos += 1;
                continue;
            }
            // emit match (updates R0/R1/R2)
            match m.rep {
                0 => coder.add_match(st, 0, 0, m.len),
                1 => {
                    std::mem::swap(&mut r0, &mut r1);
                    coder.add_match(st, 1, 0, m.len);
                }
                2 => {
                    std::mem::swap(&mut r0, &mut r2);
                    coder.add_match(st, 2, 0, m.len);
                }
                _ => {
                    r2 = r1;
                    r1 = r0;
                    r0 = m.dist;
                    let fmt = m.dist + 2;
                    let slot = slot_for(st, fmt);
                    coder.add_match(st, slot, fmt - st.base[slot], m.len);
                }
            }
            for p in pos + 1..pos + m.len {
                insert(p, head, prev);
            }
            pos += m.len;
        } else {
            coder.add_literal(data[pos]);
            insert(pos, head, prev);
            pos += 1;
        }
    }
}

struct IntervalOut {
    bytes: Vec<u8>,
    frame_ends: Vec<u64>,
}

fn compress_interval(
    st: &Slots,
    data: &[u8],
    start: usize,
    end: usize,
    head: &mut [i32],
    prev: &mut [i32],
) -> IntervalOut {
    let mut io = IntervalOut { bytes: Vec::new(), frame_ends: Vec::new() };
    let mut coder = IntervalCoder::new();
    parse_interval(st, data, start, end, &mut coder, head, prev);

    let mut main_len = [0u8; MAIN_SYMS];
    let mut len_len = [0u8; LEN_SYMS];
    let mut aligned_len = [0u8; ALIGNED_SYMS];
    let mut main_code = [0u16; MAIN_SYMS];
    let mut len_code = [0u16; LEN_SYMS];
    let mut aligned_code = [0u16; ALIGNED_SYMS];
    huff_lengths(&coder.main_freq, MAIN_SYMS, 16, &mut main_len);
    huff_lengths(&coder.len_freq, LEN_SYMS, 16, &mut len_len);
    huff_lengths(&coder.aligned_freq, ALIGNED_SYMS, 7, &mut aligned_len);
    assign_codes(&main_len, MAIN_SYMS, &mut main_code);
    assign_codes(&len_len, LEN_SYMS, &mut len_code);
    assign_codes(&aligned_len, ALIGNED_SYMS, &mut aligned_code);

    let mut raw_bits: u64 = 0;
    let mut aligned_bits: u64 = 24;
    for s in 0..ALIGNED_SYMS {
        raw_bits += 3 * coder.aligned_freq[s] as u64;
        aligned_bits += coder.aligned_freq[s] as u64 * aligned_len[s] as u64;
    }
    let block_type = if aligned_bits < raw_bits {
        BLOCK_ALIGNED
    } else {
        BLOCK_VERBATIM
    };

    let mut bw = BitWriter::new(&mut io.bytes);
    bw.put(1, 0); // E8 translation off
    bw.put(3, block_type);
    bw.put(24, (end - start) as u32);
    if block_type == BLOCK_ALIGNED {
        for s in 0..ALIGNED_SYMS {
            bw.put(3, aligned_len[s] as u32);
        }
    }
    write_tree(&mut bw, &main_len[..NUM_CHARS], NUM_CHARS);
    write_tree(&mut bw, &main_len[NUM_CHARS..], MAIN_SYMS - NUM_CHARS);
    write_tree(&mut bw, &len_len, LEN_SYMS);

    let mut in_frame: usize = 0;
    for &tok in &coder.tokens {
        let tok_bytes;
        if tok & TOK_MATCH != 0 {
            let len_m2 = (tok & 0xFF) as usize;
            let footer = (tok >> 8) & 0x1FFFF;
            let slot = ((tok >> 25) & 0x3F) as usize;
            let len_header = std::cmp::min(len_m2, 7);
            let main_sym = NUM_CHARS + (slot << 3 | len_header);
            bw.put(main_len[main_sym] as i32, main_code[main_sym] as u32);
            if len_header == 7 {
                let ls = len_m2 - 7;
                bw.put(len_len[ls] as i32, len_code[ls] as u32);
            }
            let eb = st.extra[slot] as i32;
            if block_type == BLOCK_ALIGNED && eb >= 3 {
                bw.put(eb - 3, footer >> 3);
                bw.put(
                    aligned_len[(footer & 7) as usize] as i32,
                    aligned_code[(footer & 7) as usize] as u32,
                );
            } else if eb > 0 {
                bw.put(eb, footer);
            }
            tok_bytes = len_m2 + MIN_MATCH;
        } else {
            bw.put(main_len[tok as usize] as i32, main_code[tok as usize] as u32);
            tok_bytes = 1;
        }
        in_frame += tok_bytes;
        if in_frame == FRAME {
            bw.align16();
            io.frame_ends.push(bw.out.len() as u64);
            in_frame = 0;
        }
    }
    io
}

/// Compresses `input` for a CHM MSCompressed section.
pub fn lzx_compress(input: &[u8]) -> LzxResult {
    let mut res = LzxResult {
        data: Vec::new(),
        frame_starts: Vec::new(),
        padded_size: 0,
    };
    if input.is_empty() {
        return res;
    }
    let padded = (input.len() + FRAME - 1) / FRAME * FRAME;
    res.padded_size = padded as u64;
    let mut data = vec![0u8; padded];
    data[..input.len()].copy_from_slice(input);

    let st = slot_tables();
    let n_intervals = padded.div_ceil(INTERVAL);
    let mut head = vec![-1i32; 1usize << HASH_BITS];
    let mut prev = vec![-1i32; INTERVAL];

    let mut outs: Vec<IntervalOut> = Vec::with_capacity(n_intervals);
    for i in 0..n_intervals {
        let s = i * INTERVAL;
        let e = std::cmp::min(padded, (i + 1) * INTERVAL);
        outs.push(compress_interval(&st, &data, s, e, &mut head, &mut prev));
    }

    // stitch: each interval is independently 16-bit aligned
    let total: usize = outs.iter().map(|o| o.bytes.len()).sum();
    res.data.reserve(total);
    for io in &outs {
        let base = res.data.len() as u64;
        res.frame_starts.push(base);
        for j in 0..io.frame_ends.len().saturating_sub(1) {
            res.frame_starts.push(base + io.frame_ends[j]);
        }
        res.data.extend_from_slice(&io.bytes);
    }
    res
}
