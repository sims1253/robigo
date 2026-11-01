//! Native routines of the tools package (src/library/tools/src).

use super::NativeFn;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[("Rmd5", rmd5)];

/// MD5 per RFC 1321 (glibc's md5.c, as tools uses it): little-endian,
/// 64-byte blocks, the digest as 16 bytes.
struct Md5 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    len: u64,
    block: [u8; 64],
    used: usize,
}

/// RFC 1321's per-round step: `b + rotl(a + f + k + x, s)`.
fn step(v: [u32; 4], f: u32, k: u32, w: u32, s: u32) -> [u32; 4] {
    let [a, b, c, d] = v;
    let sum = a.wrapping_add(f).wrapping_add(k).wrapping_add(w).rotate_left(s).wrapping_add(b);
    [d, sum, b, c]
}

impl Md5 {
    fn new() -> Self {
        Md5 { a: 0x6745_2301, b: 0xefcd_ab89, c: 0x98ba_dcfe, d: 0x1032_5476, len: 0, block: [0; 64], used: 0 }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = (64 - self.used).min(data.len());
            self.block[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used == 64 {
                let block = self.block;
                self.process(&block);
                self.used = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.process(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.block[..data.len()].copy_from_slice(data);
            self.used = data.len();
        }
    }

    // R: tools/src/md5.c:md5_process_block — RFC 1321's 64 steps
    fn process(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 16];
        for (k, x) in w.iter_mut().enumerate() {
            *x = u32::from_le_bytes(block[4 * k..4 * k + 4].try_into().unwrap());
        }
        let mut v = [self.a, self.b, self.c, self.d];
        const S: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
            14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10,
            15, 21, 6, 10, 15, 21,
        ];
        const K: [u32; 64] = [
            0xd76a_a478,
            0xe8c7_b756,
            0x2420_70db,
            0xc1bd_ceee,
            0xf57c_0faf,
            0x4787_c62a,
            0xa830_4613,
            0xfd46_9501,
            0x6980_98d8,
            0x8b44_f7af,
            0xffff_5bb1,
            0x895c_d7be,
            0x6b90_1122,
            0xfd98_7193,
            0xa679_438e,
            0x49b4_0821,
            0xf61e_2562,
            0xc040_b340,
            0x265e_5a51,
            0xe9b6_c7aa,
            0xd62f_105d,
            0x0244_1453,
            0xd8a1_e681,
            0xe7d3_fbc8,
            0x21e1_cde6,
            0xc337_07d6,
            0xf4d5_0d87,
            0x455a_14ed,
            0xa9e3_e905,
            0xfcef_a3f8,
            0x676f_02d9,
            0x8d2a_4c8a,
            0xfffa_3942,
            0x8771_f681,
            0x6d9d_6122,
            0xfde5_380c,
            0xa4be_ea44,
            0x4bde_cfa9,
            0xf6bb_4b60,
            0xbebf_bc70,
            0x289b_7ec6,
            0xeaa1_27fa,
            0xd4ef_3085,
            0x0488_1d05,
            0xd9d4_d039,
            0xe6db_99e5,
            0x1fa2_7cf8,
            0xc4ac_5665,
            0xf429_2244,
            0x432a_ff97,
            0xab94_23a7,
            0xfc93_a039,
            0x655b_59c3,
            0x8f0c_cc92,
            0xffef_f47d,
            0x8584_5dd1,
            0x6fa8_7e4f,
            0xfe2c_e6e0,
            0xa301_4314,
            0x4e08_11a1,
            0xf753_7e82,
            0xbd3a_f235,
            0x2ad7_d2bb,
            0xeb86_d391,
        ];
        for i in 0..16 {
            let f = (v[1] & v[2]) | (!v[1] & v[3]);
            v = step(v, f, K[i], w[i], S[i]);
        }
        for i in 16..32 {
            let f = (v[3] & v[1]) | (!v[3] & v[2]);
            v = step(v, f, K[i], w[(5 * i + 1) % 16], S[i]);
        }
        for i in 32..48 {
            let f = v[1] ^ v[2] ^ v[3];
            v = step(v, f, K[i], w[(3 * i + 5) % 16], S[i]);
        }
        for i in 48..64 {
            let f = v[2] ^ (v[1] | !v[3]);
            v = step(v, f, K[i], w[(7 * i) % 16], S[i]);
        }
        self.a = self.a.wrapping_add(v[0]);
        self.b = self.b.wrapping_add(v[1]);
        self.c = self.c.wrapping_add(v[2]);
        self.d = self.d.wrapping_add(v[3]);
    }

    fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.a.to_le_bytes());
        out[4..8].copy_from_slice(&self.b.to_le_bytes());
        out[8..12].copy_from_slice(&self.c.to_le_bytes());
        out[12..16].copy_from_slice(&self.d.to_le_bytes());
        out
    }
}

// R: tools/src/Rmd5.c:Rmd5 — the MD5 of a raw buffer, or of each file's
// contents (NA for a file that cannot be opened).
fn rmd5(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let digest = |bytes: &[u8]| {
        let mut md5 = Md5::new();
        md5.update(bytes);
        let block = md5.finish();
        let hex: String = block.iter().map(|b| format!("{b:02x}")).collect();
        hex
    };
    match &args[0].value {
        Value::Raw(x) => Ok(Value::str(&digest(&x.data))),
        Value::Str(files) => {
            let out: Vec<RStr> = files
                .data
                .iter()
                .map(|f| match f {
                    Some(path) => match std::fs::read(path.to_string()) {
                        Ok(bytes) => crate::value::rstr(&digest(&bytes)),
                        // R: Rmd5.c — a file that cannot be opened is NA
                        Err(_) => None,
                    },
                    None => None,
                })
                .collect();
            Ok(Value::Str(std::rc::Rc::new(Vector::new(out))))
        }
        _ => Err(crate::error::Flow::error("argument 'files' must be character")),
    }
}
