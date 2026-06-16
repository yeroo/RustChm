//! LZX decompressor for CHM. Port of FastChm's lzxdecode.cpp (inverse of lzx.rs).

const NUM_CHARS: usize = 256;
const PRETREE_SYMS: usize = 20;
const ALIGNED_SYMS: usize = 8;
const LEN_SYMS: usize = 249;
const MIN_MATCH: usize = 2;
const NUM_PRIMARY_LENGTHS: usize = 7;
const FRAME: u64 = 0x8000;

const EXTRA_BITS: [u8; 51] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];
const POS_BASE: [u32; 51] = [
    0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536,
    2048, 3072, 4096, 6144, 8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072, 196608,
    262144, 393216, 524288, 655360, 786432, 917504, 1048576, 1179648, 1310720, 1441792, 1572864,
    1703936, 1835008, 1966080, 2097152,
];

struct BitReader<'a> {
    p: &'a [u8],
    pos: usize,
    buf: u64,
    count: i32,
}

impl<'a> BitReader<'a> {
    fn new(p: &'a [u8]) -> Self {
        BitReader { p, pos: 0, buf: 0, count: 0 }
    }
    fn ensure(&mut self, bits: i32) {
        while self.count < bits {
            let w = if self.pos + 1 < self.p.len() {
                let v = self.p[self.pos] as u64 | ((self.p[self.pos + 1] as u64) << 8);
                self.pos += 2;
                v
            } else {
                self.pos = self.p.len();
                0
            };
            self.buf = (self.buf << 16) | w;
            self.count += 16;
        }
    }
    fn read(&mut self, bits: i32) -> u32 {
        if bits == 0 {
            return 0;
        }
        self.ensure(bits);
        let v = (self.buf >> (self.count - bits)) & ((1u64 << bits) - 1);
        self.count -= bits;
        v as u32
    }
    fn align_to_word(&mut self) {
        self.count -= self.count % 16;
    }
    fn read_bytes(&mut self, dst: &mut [u8]) {
        let mut i = 0;
        while i < dst.len() && self.count >= 8 {
            dst[i] = ((self.buf >> (self.count - 8)) & 0xFF) as u8;
            self.count -= 8;
            i += 1;
        }
        while i < dst.len() && self.pos < self.p.len() {
            dst[i] = self.p[self.pos];
            self.pos += 1;
            i += 1;
        }
        while i < dst.len() {
            dst[i] = 0;
            i += 1;
        }
    }
}

struct HuffDecoder {
    count: [u32; 18],
    first_code: [u32; 18],
    first_index: [u32; 18],
    max_excl: [u32; 18],
    syms: Vec<u16>,
}

impl HuffDecoder {
    fn new() -> Self {
        HuffDecoder {
            count: [0; 18],
            first_code: [0; 18],
            first_index: [0; 18],
            max_excl: [0; 18],
            syms: Vec::new(),
        }
    }
    fn build(&mut self, lens: &[u8], n: usize) {
        self.count = [0; 18];
        for &l in lens.iter().take(n) {
            self.count[l as usize] += 1;
        }
        let mut code = 0u32;
        let mut idx = 0u32;
        for l in 1..=16 {
            self.first_code[l] = code;
            self.first_index[l] = idx;
            idx += self.count[l];
            self.max_excl[l] = code + self.count[l];
            code = (code + self.count[l]) << 1;
        }
        self.syms = vec![0u16; idx as usize];
        let mut next = [0u32; 18];
        for l in 1..=16 {
            next[l] = self.first_index[l];
        }
        for (s, &l) in lens.iter().take(n).enumerate() {
            if l != 0 {
                self.syms[next[l as usize] as usize] = s as u16;
                next[l as usize] += 1;
            }
        }
    }
    fn decode(&self, br: &mut BitReader) -> i32 {
        let mut code = 0u32;
        for l in 1..=16 {
            code = (code << 1) | br.read(1);
            if self.count[l] != 0 && code >= self.first_code[l] && code < self.max_excl[l] {
                return self.syms[(self.first_index[l] + (code - self.first_code[l])) as usize] as i32;
            }
        }
        -1
    }
}

fn read_lengths(br: &mut BitReader, lens: &mut [u8], size: usize) -> bool {
    let mut pre_len = [0u8; PRETREE_SYMS];
    for x in pre_len.iter_mut() {
        *x = br.read(4) as u8;
    }
    let mut pre = HuffDecoder::new();
    pre.build(&pre_len, PRETREE_SYMS);
    let mut i = 0;
    while i < size {
        let sym = pre.decode(br);
        if sym < 0 {
            return false;
        }
        match sym {
            17 => {
                let mut run = br.read(4) + 4;
                while run > 0 && i < size {
                    lens[i] = 0;
                    i += 1;
                    run -= 1;
                }
            }
            18 => {
                let mut run = br.read(5) + 20;
                while run > 0 && i < size {
                    lens[i] = 0;
                    i += 1;
                    run -= 1;
                }
            }
            19 => {
                let mut run = br.read(1) + 4;
                let v = pre.decode(br);
                if v < 0 {
                    return false;
                }
                let new_len = ((lens[i] as i32 - v + 17) % 17) as u8;
                while run > 0 && i < size {
                    lens[i] = new_len;
                    i += 1;
                    run -= 1;
                }
            }
            _ => {
                lens[i] = ((lens[i] as i32 - sym + 17) % 17) as u8;
                i += 1;
            }
        }
    }
    true
}

pub fn lzx_decompress(
    comp: &[u8],
    uncompressed_size: u64,
    mut reset_interval: u32,
    window_bits: u32,
) -> Result<Vec<u8>, String> {
    static SLOTS: [usize; 7] = [30, 32, 34, 36, 38, 42, 50];
    if !(15..=21).contains(&window_bits) {
        return Err("unsupported LZX window".into());
    }
    let num_slots = SLOTS[(window_bits - 15) as usize];
    let main_syms = NUM_CHARS + 8 * num_slots;
    if reset_interval == 0 {
        reset_interval = 0x10000;
    }

    let mut out: Vec<u8> = Vec::with_capacity(uncompressed_size as usize);
    let mut br = BitReader::new(comp);
    let (mut r0, mut r1, mut r2) = (1i32, 1i32, 1i32);
    let mut main_len = vec![0u8; main_syms];
    let mut len_len = vec![0u8; LEN_SYMS];
    let mut align_len = vec![0u8; ALIGNED_SYMS];
    let mut main_tree = HuffDecoder::new();
    let mut len_tree = HuffDecoder::new();
    let mut align_tree = HuffDecoder::new();

    let mut produced: u64 = 0;
    let mut interval_remaining: u64 = 0;
    let mut since_frame: u64 = 0;
    let mut need_reset = true;
    let mut block_type = 0u32;

    while produced < uncompressed_size {
        if need_reset {
            r0 = 1;
            r1 = 1;
            r2 = 1;
            main_len.iter_mut().for_each(|x| *x = 0);
            len_len.iter_mut().for_each(|x| *x = 0);
            br.read(1); // E8 flag
            need_reset = false;
            interval_remaining = 0;
        }
        if interval_remaining == 0 {
            block_type = br.read(3);
            let block_len = br.read(24) as u64;
            interval_remaining = block_len;
            if block_type == 1 || block_type == 2 {
                if block_type == 2 {
                    for x in align_len.iter_mut() {
                        *x = br.read(3) as u8;
                    }
                }
                if !read_lengths(&mut br, &mut main_len, NUM_CHARS)
                    || !read_lengths(&mut br, &mut main_len[NUM_CHARS..], main_syms - NUM_CHARS)
                    || !read_lengths(&mut br, &mut len_len, LEN_SYMS)
                {
                    return Err("corrupt LZX trees".into());
                }
                main_tree.build(&main_len, main_syms);
                len_tree.build(&len_len, LEN_SYMS);
                if block_type == 2 {
                    align_tree.build(&align_len, ALIGNED_SYMS);
                }
            } else if block_type == 3 {
                br.align_to_word();
                let mut r = [0u8; 12];
                br.read_bytes(&mut r);
                r0 = i32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                r1 = i32::from_le_bytes([r[4], r[5], r[6], r[7]]);
                r2 = i32::from_le_bytes([r[8], r[9], r[10], r[11]]);
            } else {
                return Err("bad LZX block type".into());
            }
        }

        if block_type == 3 {
            let chunk = interval_remaining
                .min(FRAME - since_frame)
                .min(uncompressed_size - produced) as usize;
            let base = out.len();
            out.resize(base + chunk, 0);
            br.read_bytes(&mut out[base..]);
            produced += chunk as u64;
            interval_remaining -= chunk as u64;
            since_frame += chunk as u64;
        } else {
            let sym = main_tree.decode(&mut br);
            if sym < 0 {
                return Err("corrupt LZX stream".into());
            }
            if (sym as usize) < NUM_CHARS {
                out.push(sym as u8);
                produced += 1;
                interval_remaining -= 1;
                since_frame += 1;
            } else {
                let slot = (sym as usize - NUM_CHARS) >> 3;
                let len_header = (sym as usize - NUM_CHARS) & 7;
                let mut match_len = len_header + MIN_MATCH;
                if len_header == NUM_PRIMARY_LENGTHS {
                    let extra = len_tree.decode(&mut br);
                    if extra < 0 {
                        return Err("corrupt LZX length".into());
                    }
                    match_len = extra as usize + NUM_PRIMARY_LENGTHS + MIN_MATCH;
                }
                let match_offset: i32;
                if slot == 0 {
                    match_offset = r0;
                } else if slot == 1 {
                    match_offset = r1;
                    r1 = r0;
                    r0 = match_offset;
                } else if slot == 2 {
                    match_offset = r2;
                    r2 = r0;
                    r0 = match_offset;
                } else {
                    let eb = EXTRA_BITS[slot] as i32;
                    let footer = if block_type == 2 && eb >= 3 {
                        let verbatim = if eb > 3 { br.read(eb - 3) } else { 0 };
                        let a = align_tree.decode(&mut br);
                        if a < 0 {
                            return Err("corrupt LZX aligned".into());
                        }
                        (verbatim << 3) | a as u32
                    } else {
                        br.read(eb)
                    };
                    match_offset = (POS_BASE[slot] + footer) as i32 - 2;
                    r2 = r1;
                    r1 = r0;
                    r0 = match_offset;
                }
                if match_offset <= 0 || match_offset as usize > out.len() {
                    return Err("LZX match out of range".into());
                }
                let src = out.len() - match_offset as usize;
                for k in 0..match_len {
                    let b = out[src + k];
                    out.push(b);
                }
                produced += match_len as u64;
                interval_remaining -= match_len as u64;
                since_frame += match_len as u64;
            }
        }

        if since_frame >= FRAME {
            br.align_to_word();
            since_frame = 0;
            if produced % reset_interval as u64 == 0 {
                need_reset = true;
            }
        }
    }
    out.truncate(uncompressed_size as usize);
    Ok(out)
}
