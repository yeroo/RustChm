//! Little-endian byte buffer helpers and the ITSS variable-length integer (ENCINT).

#[derive(Default)]
pub struct Buf {
    pub v: Vec<u8>,
}

impl Buf {
    pub fn new() -> Self {
        Buf { v: Vec::new() }
    }
    pub fn u8(&mut self, x: u8) {
        self.v.push(x);
    }
    pub fn u16(&mut self, x: u16) {
        self.v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn u32(&mut self, x: u32) {
        self.v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn i32(&mut self, x: i32) {
        self.v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn u64(&mut self, x: u64) {
        self.v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn raw(&mut self, b: &[u8]) {
        self.v.extend_from_slice(b);
    }
    pub fn str(&mut self, s: &str) {
        self.v.extend_from_slice(s.as_bytes());
    }
    pub fn strz(&mut self, s: &str) {
        self.str(s);
        self.u8(0);
    }
    pub fn zeros(&mut self, n: usize) {
        self.v.resize(self.v.len() + n, 0);
    }
    /// GUID layout: DWORD, WORD, WORD, BYTE[8].
    pub fn guid(&mut self, a: u32, b: u16, c: u16, d: &[u8; 8]) {
        self.u32(a);
        self.u16(b);
        self.u16(c);
        self.raw(d);
    }
    pub fn len(&self) -> usize {
        self.v.len()
    }
    pub fn is_empty(&self) -> bool {
        self.v.is_empty()
    }
}

/// ITSS variable-length integer: 7 bits per byte, most-significant group first,
/// high bit set on all but the last byte.
pub fn encint(out: &mut Vec<u8>, mut x: u64) {
    let mut tmp = [0u8; 10];
    let mut n = 0;
    loop {
        tmp[n] = (x & 0x7F) as u8;
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    for i in (1..n).rev() {
        out.push(tmp[i] | 0x80);
    }
    out.push(tmp[0]);
}

/// Length in bytes that `encint` would emit for `x`.
pub fn encint_len(mut x: u64) -> usize {
    let mut n = 0;
    loop {
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    n
}
