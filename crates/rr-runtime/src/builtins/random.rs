//! Random number generation (GNU R's RNG.c). Placeholder until the
//! Mersenne-Twister port lands.

use crate::value::Builtin;

#[derive(Default)]
pub struct Rng {
    pub seeded: bool,
}

pub static DEFS: &[Builtin] = &[];
