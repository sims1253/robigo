//! Uniform random number generators and the standard normal and
//! exponential generators: GNU R's main/RNG.c, nmath/snorm.c, sexp.c.
//!
//! As in R, all generators share one seed buffer (`dummy` in RNG.c);
//! `.Random.seed` is the generator code followed by the first
//! `n_seed(kind)` words of it.

use super::norm::qnorm;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RngKind {
    WichmannHill = 0,
    MarsagliaMulticarry = 1,
    SuperDuper = 2,
    MersenneTwister = 3,
    KnuthTaocp = 4,
    UserUnif = 5,
    KnuthTaocp2 = 6,
    LecuyerCmrg = 7,
}

impl RngKind {
    pub fn from_code(i: i32) -> Option<RngKind> {
        Some(match i {
            0 => RngKind::WichmannHill,
            1 => RngKind::MarsagliaMulticarry,
            2 => RngKind::SuperDuper,
            3 => RngKind::MersenneTwister,
            4 => RngKind::KnuthTaocp,
            5 => RngKind::UserUnif,
            6 => RngKind::KnuthTaocp2,
            7 => RngKind::LecuyerCmrg,
            _ => return None,
        })
    }

    /// Length of the seed vector (`RNG_Table[kind].n_seed`).
    pub fn n_seed(self) -> usize {
        match self {
            RngKind::WichmannHill => 3,
            RngKind::MarsagliaMulticarry => 2,
            RngKind::SuperDuper => 2,
            RngKind::MersenneTwister => 1 + 624,
            RngKind::KnuthTaocp => 1 + 100,
            RngKind::UserUnif => 0,
            RngKind::KnuthTaocp2 => 1 + 100,
            RngKind::LecuyerCmrg => 6,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum N01Kind {
    BuggyKindermanRamage = 0,
    AhrensDieter = 1,
    BoxMuller = 2,
    UserNorm = 3,
    Inversion = 4,
    KindermanRamage = 5,
}

impl N01Kind {
    pub fn from_code(i: i32) -> Option<N01Kind> {
        Some(match i {
            0 => N01Kind::BuggyKindermanRamage,
            1 => N01Kind::AhrensDieter,
            2 => N01Kind::BoxMuller,
            3 => N01Kind::UserNorm,
            4 => N01Kind::Inversion,
            5 => N01Kind::KindermanRamage,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleKind {
    Rounding = 0,
    Rejection = 1,
}

const I2_32M1: f64 = 2.328306437080797e-10; // 1/(2^32 - 1)
const KT: f64 = 9.31322574615479e-10; // 2^-30

// Mersenne-Twister parameters
const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908b0df;
const UPPER_MASK: u32 = 0x80000000;
const LOWER_MASK: u32 = 0x7fffffff;
const TEMPERING_MASK_B: u32 = 0x9d2c5680;
const TEMPERING_MASK_C: u32 = 0xefc60000;

// Knuth TAOCP parameters
const KK: usize = 100;
const LL: usize = 37;
const MM: u32 = 1 << 30;
const TT: i32 = 70;
const QUALITY: usize = 1009;

// L'Ecuyer-CMRG parameters
const LM1: i64 = 4294967087;
const LM2: i64 = 4294944443;
const NORMC: f64 = 2.328306549295727688e-10;
const A12: i64 = 1403580;
const A13N: i64 = 810728;
const A21: i64 = 527612;
const A23N: i64 = 1370589;

#[inline]
fn mod_diff(x: u32, y: u32) -> u32 {
    x.wrapping_sub(y) & (MM - 1)
}

/// Request to run R-level code during seeding: Knuth-TAOCP (1997) is
/// initialised by the R function `.TAOCP1997init(seed)`.
#[derive(Debug)]
pub struct NeedTaocp1997Init(pub i32);

pub struct Rng {
    pub kind: RngKind,
    pub n01_kind: N01Kind,
    pub sample_kind: SampleKind,
    /// Box-Muller's saved second deviate.
    pub bm_norm_keep: f64,
    /// R's `dummy`: the seed buffer shared by all generators.
    pub seed: [u32; 628],
    kt_arr_buf: Vec<u32>,
}

impl Default for Rng {
    fn default() -> Self {
        Rng {
            kind: RngKind::MersenneTwister,
            n01_kind: N01Kind::Inversion,
            sample_kind: SampleKind::Rejection,
            bm_norm_keep: 0.0,
            seed: [0; 628],
            kt_arr_buf: vec![0; QUALITY],
        }
    }
}

/// R: main/times.c:TimeToSeed
pub fn time_to_seed() -> u32 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let mut seed = (((now.subsec_nanos() as u64) << 16) ^ now.as_secs()) as u32;
    let pid = std::process::id();
    seed ^= pid << 16;
    seed
}

impl Rng {
    fn fixup(x: f64) -> f64 {
        if x <= 0.0 {
            return 0.5 * I2_32M1;
        }
        if (1.0 - x) <= 0.0 {
            return 1.0 - 0.5 * I2_32M1;
        }
        x
    }

    /// R: main/RNG.c:unif_rand
    pub fn unif_rand(&mut self) -> f64 {
        let s = &mut self.seed;
        match self.kind {
            RngKind::WichmannHill => {
                s[0] = s[0].wrapping_mul(171) % 30269;
                s[1] = s[1].wrapping_mul(172) % 30307;
                s[2] = s[2].wrapping_mul(170) % 30323;
                let value = s[0] as f64 / 30269.0 + s[1] as f64 / 30307.0 + s[2] as f64 / 30323.0;
                Self::fixup(value - (value as i32) as f64)
            }
            RngKind::MarsagliaMulticarry => {
                s[0] = 36969u32.wrapping_mul(s[0] & 0o177777).wrapping_add(s[0] >> 16);
                s[1] = 18000u32.wrapping_mul(s[1] & 0o177777).wrapping_add(s[1] >> 16);
                Self::fixup(((s[0] << 16) ^ (s[1] & 0o177777)) as f64 * I2_32M1)
            }
            RngKind::SuperDuper => {
                s[0] ^= (s[0] >> 15) & 0o377777;
                s[0] ^= s[0] << 17;
                s[1] = s[1].wrapping_mul(69069);
                Self::fixup((s[0] ^ s[1]) as f64 * I2_32M1)
            }
            RngKind::MersenneTwister => {
                let v = self.mt_genrand();
                Self::fixup(v)
            }
            RngKind::KnuthTaocp | RngKind::KnuthTaocp2 => {
                let v = self.kt_next();
                Self::fixup(v as f64 * KT)
            }
            RngKind::UserUnif => {
                // Unreachable: selecting it fails ("'user_unif_rand' not in load table").
                f64::NAN
            }
            RngKind::LecuyerCmrg => {
                let ii = |s: &[u32; 628], i: usize| s[i] as i64;
                let mut p1 = A12 * ii(s, 1) - A13N * ii(s, 0);
                let k = (p1 / LM1) as i32;
                p1 -= k as i64 * LM1;
                if p1 < 0 {
                    p1 += LM1;
                }
                s[0] = s[1];
                s[1] = s[2];
                s[2] = p1 as i32 as u32;

                let mut p2 = A21 * ii(s, 5) - A23N * ii(s, 3);
                let k = (p2 / LM2) as i32;
                p2 -= k as i64 * LM2;
                if p2 < 0 {
                    p2 += LM2;
                }
                s[3] = s[4];
                s[4] = s[5];
                s[5] = p2 as i32 as u32;

                (if p1 > p2 { p1 - p2 } else { p1 - p2 + LM1 }) as f64 * NORMC
            }
        }
    }

    /// R: main/RNG.c:FixupSeeds. Returns true when the seeds are all
    /// zero and the generator must be re-randomized (`Randomize(kind)`).
    pub fn fixup_seeds(&mut self, kind: RngKind, initial: bool) -> bool {
        let s = &mut self.seed;
        match kind {
            RngKind::WichmannHill => {
                s[0] %= 30269;
                s[1] %= 30307;
                s[2] %= 30323;
                if s[0] == 0 {
                    s[0] = 1;
                }
                if s[1] == 0 {
                    s[1] = 1;
                }
                if s[2] == 0 {
                    s[2] = 1;
                }
                false
            }
            RngKind::SuperDuper => {
                if s[0] == 0 {
                    s[0] = 1;
                }
                s[1] |= 1;
                false
            }
            RngKind::MarsagliaMulticarry => {
                if s[0] == 0 {
                    s[0] = 1;
                }
                if s[1] == 0 {
                    s[1] = 1;
                }
                false
            }
            RngKind::MersenneTwister => {
                if initial {
                    s[0] = 624;
                }
                // I1 is unsigned in C: only 0 is "<= 0"
                if s[0] == 0 {
                    s[0] = 624;
                }
                !s[1..=624].iter().any(|&v| v != 0)
            }
            RngKind::KnuthTaocp | RngKind::KnuthTaocp2 => {
                if s[100] == 0 {
                    s[100] = 100;
                }
                !s[0..100].iter().any(|&v| v != 0)
            }
            RngKind::UserUnif => false,
            RngKind::LecuyerCmrg => {
                let mut notallzero = false;
                let mut all_ok = true;
                for &t in &s[0..3] {
                    if t != 0 {
                        notallzero = true;
                    }
                    if t as i64 >= LM1 {
                        all_ok = false;
                    }
                }
                if !notallzero || !all_ok {
                    return true;
                }
                for &t in &s[3..6] {
                    if t != 0 {
                        notallzero = true;
                    }
                    if t as i64 >= LM2 {
                        all_ok = false;
                    }
                }
                !notallzero || !all_ok
            }
        }
    }

    /// R: main/RNG.c:RNG_Init. `Err` asks the caller to run
    /// `.TAOCP1997init(seed)` and pass the result to `set_taocp1997`.
    pub fn init(&mut self, kind: RngKind, mut seed: u32) -> Result<(), NeedTaocp1997Init> {
        self.bm_norm_keep = 0.0;
        for _ in 0..50 {
            seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
        }
        match kind {
            RngKind::WichmannHill | RngKind::MarsagliaMulticarry | RngKind::SuperDuper | RngKind::MersenneTwister => {
                for j in 0..kind.n_seed() {
                    seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
                    self.seed[j] = seed;
                }
                if self.fixup_seeds(kind, true) {
                    return self.init(kind, time_to_seed());
                }
                Ok(())
            }
            RngKind::KnuthTaocp => Err(NeedTaocp1997Init((seed % 1073741821) as i32)),
            RngKind::KnuthTaocp2 => {
                self.ran_start(seed % 1073741821);
                self.seed[100] = 100;
                Ok(())
            }
            RngKind::LecuyerCmrg => {
                for j in 0..kind.n_seed() {
                    seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
                    while seed as i64 >= LM2 {
                        seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
                    }
                    self.seed[j] = seed;
                }
                Ok(())
            }
            RngKind::UserUnif => Ok(()),
        }
    }

    /// Finish Knuth-TAOCP (1997) seeding with the 100 words computed by
    /// `.TAOCP1997init`.
    pub fn set_taocp1997(&mut self, words: &[i32]) {
        for (d, &w) in self.seed.iter_mut().zip(words.iter().take(100)) {
            *d = w as u32;
        }
        self.seed[100] = 100;
    }

    // ----- Mersenne-Twister ---------------------------------------------

    fn mt_sgenrand(&mut self, mut seed: u32) {
        let mt = &mut self.seed[1..=N];
        for v in mt.iter_mut() {
            *v = seed & 0xffff0000;
            seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
            *v |= (seed & 0xffff0000) >> 16;
            seed = 69069u32.wrapping_mul(seed).wrapping_add(1);
        }
        self.seed[0] = N as u32;
    }

    /// R: main/RNG.c:MT_genrand
    fn mt_genrand(&mut self) -> f64 {
        const MAG01: [u32; 2] = [0x0, MATRIX_A];
        // C reads `mti = dummy[0]` into an int; a negative value would
        // index out of bounds there, we regenerate instead.
        let mut mti = self.seed[0] as i32 as i64;
        if mti >= N as i64 || mti < 0 {
            if mti == (N + 1) as i64 {
                self.mt_sgenrand(4357);
            }
            let mt = &mut self.seed[1..=N];
            let mut kk = 0;
            while kk < N - M {
                let y = (mt[kk] & UPPER_MASK) | (mt[kk + 1] & LOWER_MASK);
                mt[kk] = mt[kk + M] ^ (y >> 1) ^ MAG01[(y & 0x1) as usize];
                kk += 1;
            }
            while kk < N - 1 {
                let y = (mt[kk] & UPPER_MASK) | (mt[kk + 1] & LOWER_MASK);
                mt[kk] = mt[kk + M - N] ^ (y >> 1) ^ MAG01[(y & 0x1) as usize];
                kk += 1;
            }
            let y = (mt[N - 1] & UPPER_MASK) | (mt[0] & LOWER_MASK);
            mt[N - 1] = mt[M - 1] ^ (y >> 1) ^ MAG01[(y & 0x1) as usize];
            mti = 0;
        }
        let mut y = self.seed[1 + mti as usize];
        mti += 1;
        y ^= y >> 11;
        y ^= (y << 7) & TEMPERING_MASK_B;
        y ^= (y << 15) & TEMPERING_MASK_C;
        y ^= y >> 18;
        self.seed[0] = mti as u32;
        y as f64 * 2.3283064365386963e-10
    }

    // ----- Knuth TAOCP ----------------------------------------------------

    fn ran_array(&mut self, aa: &mut [u32], n: usize) {
        let ran_x = &mut self.seed[0..KK];
        let mut j = 0;
        while j < KK {
            aa[j] = ran_x[j];
            j += 1;
        }
        while j < n {
            aa[j] = mod_diff(aa[j - KK], aa[j - LL]);
            j += 1;
        }
        let mut i = 0;
        while i < LL {
            ran_x[i] = mod_diff(aa[j - KK], aa[j - LL]);
            i += 1;
            j += 1;
        }
        while i < KK {
            ran_x[i] = mod_diff(aa[j - KK], ran_x[i - LL]);
            i += 1;
            j += 1;
        }
    }

    fn ran_arr_cycle(&mut self) {
        let mut buf = std::mem::take(&mut self.kt_arr_buf);
        self.ran_array(&mut buf, QUALITY);
        buf[KK] = u32::MAX;
        self.kt_arr_buf = buf;
    }

    fn ran_start(&mut self, seed: u32) {
        let mut x = [0u32; KK + KK - 1];
        let mut ss = seed.wrapping_add(2) & (MM - 2);
        for v in x.iter_mut().take(KK) {
            *v = ss;
            ss <<= 1;
            if ss >= MM {
                ss -= MM - 2;
            }
        }
        x[1] += 1;
        ss = seed & (MM - 1);
        let mut t = TT - 1;
        while t != 0 {
            let mut j = KK - 1;
            while j > 0 {
                x[j + j] = x[j];
                x[j + j - 1] = 0;
                j -= 1;
            }
            let mut j = KK + KK - 2;
            while j >= KK {
                x[j - (KK - LL)] = mod_diff(x[j - (KK - LL)], x[j]);
                x[j - KK] = mod_diff(x[j - KK], x[j]);
                j -= 1;
            }
            if ss & 1 != 0 {
                let mut j = KK;
                while j > 0 {
                    x[j] = x[j - 1];
                    j -= 1;
                }
                x[0] = x[KK];
                x[LL] = mod_diff(x[LL], x[KK]);
            }
            if ss != 0 {
                ss >>= 1;
            } else {
                t -= 1;
            }
        }
        let mut j = 0;
        while j < LL {
            self.seed[j + KK - LL] = x[j];
            j += 1;
        }
        while j < KK {
            self.seed[j - LL] = x[j];
            j += 1;
        }
        for _ in 0..10 {
            self.ran_array(&mut x, KK + KK - 1);
        }
    }

    fn kt_next(&mut self) -> u32 {
        if self.seed[100] >= 100 {
            self.ran_arr_cycle();
            self.seed[100] = 0;
        }
        let pos = self.seed[100] as usize;
        self.seed[100] += 1;
        self.seed[pos]
    }

    // ----- integer sampling ---------------------------------------------

    fn ru(&mut self) -> f64 {
        let u = 33554432.0;
        ((u * self.unif_rand()).floor() + self.unif_rand()) / u
    }

    fn unif_index_0(&mut self, dn: f64) -> f64 {
        let cut = match self.kind {
            RngKind::KnuthTaocp | RngKind::UserUnif | RngKind::KnuthTaocp2 => 33554431.0,
            _ => INT_MAX as f64,
        };
        let u = if dn > cut { self.ru() } else { self.unif_rand() };
        (dn * u).floor()
    }

    fn rbits(&mut self, bits: i32) -> f64 {
        let mut v: i64 = 0;
        let mut n = 0;
        while n <= bits {
            let v1 = (self.unif_rand() * 65536.0).floor() as i32;
            v = 65536i64.wrapping_mul(v).wrapping_add(v1 as i64);
            n += 16;
        }
        (v & ((1i64 << bits) - 1)) as f64
    }

    /// R: main/RNG.c:R_unif_index -- uniform on 0..dn-1
    pub fn unif_index(&mut self, dn: f64) -> f64 {
        if self.sample_kind == SampleKind::Rounding {
            return self.unif_index_0(dn);
        }
        if dn <= 0.0 {
            return 0.0;
        }
        let bits = dn.log2().ceil() as i32;
        loop {
            let dv = self.rbits(bits);
            if dn > dv {
                return dv;
            }
        }
    }

    // ----- normal and exponential ----------------------------------------

    /// R: nmath/snorm.c:norm_rand
    pub fn norm_rand(&mut self) -> f64 {
        const A: [f64; 32] = [
            0.0000000, 0.03917609, 0.07841241, 0.1177699, 0.1573107, 0.19709910, 0.23720210, 0.2776904, 0.3186394,
            0.36012990, 0.40225010, 0.4450965, 0.4887764, 0.53340970, 0.57913220, 0.6260990, 0.6744898, 0.72451440,
            0.77642180, 0.8305109, 0.8871466, 0.94678180, 1.00999000, 1.0775160, 1.1503490, 1.22985900, 1.31801100,
            1.4177970, 1.5341210, 1.67594000, 1.86273200, 2.1538750,
        ];
        const D: [f64; 31] = [
            0.0000000, 0.0000000, 0.0000000, 0.0000000, 0.0000000, 0.2636843, 0.2425085, 0.2255674, 0.2116342,
            0.1999243, 0.1899108, 0.1812252, 0.1736014, 0.1668419, 0.1607967, 0.1553497, 0.1504094, 0.1459026,
            0.1417700, 0.1379632, 0.1344418, 0.1311722, 0.1281260, 0.1252791, 0.1226109, 0.1201036, 0.1177417,
            0.1155119, 0.1134023, 0.1114027, 0.1095039,
        ];
        const T: [f64; 31] = [
            7.673828e-4, 0.002306870, 0.003860618, 0.005438454, 0.007050699, 0.008708396, 0.010423570, 0.012209530,
            0.014081250, 0.016055790, 0.018152900, 0.020395730, 0.022811770, 0.025434070, 0.028302960, 0.031468220,
            0.034992330, 0.038954830, 0.043458780, 0.048640350, 0.054683340, 0.061842220, 0.070479830, 0.081131950,
            0.094624440, 0.112300100, 0.136498000, 0.171688600, 0.227624100, 0.330498000, 0.584703100,
        ];
        const H: [f64; 31] = [
            0.03920617, 0.03932705, 0.03950999, 0.03975703, 0.04007093, 0.04045533, 0.04091481, 0.04145507,
            0.04208311, 0.04280748, 0.04363863, 0.04458932, 0.04567523, 0.04691571, 0.04833487, 0.04996298,
            0.05183859, 0.05401138, 0.05654656, 0.05953130, 0.06308489, 0.06737503, 0.07264544, 0.07926471,
            0.08781922, 0.09930398, 0.11555990, 0.14043440, 0.18361420, 0.27900160, 0.70104740,
        ];
        const C1: f64 = 0.398942280401433;
        const C2: f64 = 0.180025191068563;
        const AA: f64 = 2.216035867166471;
        let g = |x: f64| C1 * (-x * x / 2.0).exp() - C2 * (AA - x);
        const BIG: f64 = 134217728.0;

        match self.n01_kind {
            N01Kind::AhrensDieter => {
                let mut u1 = self.unif_rand();
                let mut s = 0.0;
                if u1 > 0.5 {
                    s = 1.0;
                }
                u1 = u1 + u1 - s;
                u1 *= 32.0;
                let mut i = u1 as i32;
                if i == 32 {
                    i = 31;
                }
                let aa;
                let mut w;
                if i != 0 {
                    let mut u2 = u1 - i as f64;
                    aa = A[(i - 1) as usize];
                    'outer: {
                        while u2 <= T[(i - 1) as usize] {
                            u1 = self.unif_rand();
                            w = u1 * (A[i as usize] - aa);
                            let mut tt = (w * 0.5 + aa) * w;
                            loop {
                                if u2 > tt {
                                    break 'outer; // deliver
                                }
                                u1 = self.unif_rand();
                                if u2 < u1 {
                                    break;
                                }
                                tt = u1;
                                u2 = self.unif_rand();
                            }
                            u2 = self.unif_rand();
                        }
                        w = (u2 - T[(i - 1) as usize]) * H[(i - 1) as usize];
                    }
                } else {
                    let mut i = 6usize;
                    let mut a = A[31];
                    loop {
                        u1 += u1;
                        if u1 >= 1.0 {
                            break;
                        }
                        a += D[i - 1];
                        i += 1;
                    }
                    aa = a;
                    u1 -= 1.0;
                    'jump: loop {
                        w = u1 * D[i - 1];
                        let mut tt = (w * 0.5 + aa) * w;
                        loop {
                            let u2 = self.unif_rand();
                            if u2 > tt {
                                break 'jump;
                            }
                            u1 = self.unif_rand();
                            if u2 < u1 {
                                break;
                            }
                            tt = u1;
                        }
                        u1 = self.unif_rand();
                    }
                }
                let y = aa + w;
                if s == 1.0 {
                    -y
                } else {
                    y
                }
            }
            N01Kind::BuggyKindermanRamage => self.kinderman_ramage(true, &g),
            N01Kind::KindermanRamage => self.kinderman_ramage(false, &g),
            N01Kind::BoxMuller => {
                if self.bm_norm_keep != 0.0 {
                    let s = self.bm_norm_keep;
                    self.bm_norm_keep = 0.0;
                    s
                } else {
                    let theta = 2.0 * M_PI * self.unif_rand();
                    let r = (-2.0 * self.unif_rand().ln()).sqrt() + 10.0 * DBL_MIN;
                    self.bm_norm_keep = r * theta.sin();
                    r * theta.cos()
                }
            }
            N01Kind::UserNorm => f64::NAN,
            N01Kind::Inversion => {
                let mut u1 = self.unif_rand();
                u1 = ((BIG * u1) as i32) as f64 + self.unif_rand();
                qnorm(u1 / BIG, 0.0, 1.0, true, false)
            }
        }
    }

    fn kinderman_ramage(&mut self, buggy: bool, g: &dyn Fn(f64) -> f64) -> f64 {
        const AA: f64 = 2.216035867166471;
        let u1 = self.unif_rand();
        if u1 < 0.884070402298758 {
            let u2 = self.unif_rand();
            let c = if buggy { 1.13113163544180 } else { 1.131131635444180 };
            return AA * (c * u1 + u2 - 1.0);
        }
        if u1 >= 0.973310954173898 {
            loop {
                let u2 = self.unif_rand();
                let u3 = self.unif_rand();
                let tt = AA * AA - 2.0 * u3.ln();
                if u2 * u2 < (AA * AA) / tt {
                    return if u1 < 0.986655477086949 { tt.sqrt() } else { -tt.sqrt() };
                }
            }
        }
        if u1 >= 0.958720824790463 {
            loop {
                let u2 = self.unif_rand();
                let u3 = self.unif_rand();
                let tt = AA - 0.630834801921960 * fmin2(u2, u3);
                if fmax2(u2, u3) <= 0.755591531667601 {
                    return if u2 < u3 { tt } else { -tt };
                }
                if 0.034240503750111 * (u2 - u3).abs() <= g(tt) {
                    return if u2 < u3 { tt } else { -tt };
                }
            }
        }
        if u1 >= 0.911312780288703 {
            loop {
                let u2 = self.unif_rand();
                let u3 = self.unif_rand();
                let tt = 0.479727404222441 + 1.105473661022070 * fmin2(u2, u3);
                if fmax2(u2, u3) <= 0.872834976671790 {
                    return if u2 < u3 { tt } else { -tt };
                }
                if 0.049264496373128 * (u2 - u3).abs() <= g(tt) {
                    return if u2 < u3 { tt } else { -tt };
                }
            }
        }
        loop {
            let u2 = self.unif_rand();
            let u3 = self.unif_rand();
            let tt = 0.479727404222441 - 0.595507138015940 * fmin2(u2, u3);
            if buggy {
                if fmax2(u2, u3) <= 0.805577924423817 {
                    return if u2 < u3 { tt } else { -tt };
                }
            } else {
                if tt < 0.0 {
                    continue;
                }
                if fmax2(u2, u3) <= 0.805577924423817 {
                    return if u2 < u3 { tt } else { -tt };
                }
                if 0.053377549506886 * (u2 - u3).abs() <= g(tt) {
                    return if u2 < u3 { tt } else { -tt };
                }
            }
        }
    }

    /// R: nmath/sexp.c:exp_rand
    pub fn exp_rand(&mut self) -> f64 {
        const Q: [f64; 16] = [
            0.6931471805599453,
            0.9333736875190459,
            0.9888777961838675,
            0.9984959252914960,
            0.9998292811061389,
            0.9999833164100727,
            0.9999985691438767,
            0.9999998906925558,
            0.9999999924734159,
            0.9999999995283275,
            0.9999999999728814,
            0.9999999999985598,
            0.9999999999999289,
            0.9999999999999968,
            0.9999999999999999,
            1.0000000000000000,
        ];
        let mut a = 0.0;
        let mut u = self.unif_rand();
        while u <= 0.0 || u >= 1.0 {
            u = self.unif_rand();
        }
        loop {
            u += u;
            if u > 1.0 {
                break;
            }
            a += Q[0];
        }
        u -= 1.0;
        if u <= Q[0] {
            return a + u;
        }
        let mut i = 0;
        let mut ustar = self.unif_rand();
        let mut umin = ustar;
        loop {
            ustar = self.unif_rand();
            if umin > ustar {
                umin = ustar;
            }
            i += 1;
            if !(u > Q[i]) {
                break;
            }
        }
        a + umin * Q[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt_stream_matches_r() {
        // set.seed(42); sprintf("%.17g", runif(3)); .Random.seed[2:3]
        let mut r = Rng::default();
        r.init(RngKind::MersenneTwister, 42).unwrap();
        assert_eq!(r.unif_rand(), 0.91480604349635541);
        assert_eq!(r.unif_rand(), 0.93707541329786181);
        assert_eq!(r.unif_rand(), 0.28613953478634357);
    }
}
