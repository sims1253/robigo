//! The cost of `Core::install`, a hit and a miss, for callgrind:
//! `valgrind --tool=callgrind --toggle-collect='install_cost::hits' ...`
//! (and `misses`); the instructions over the count are one call's.

use robigo_heap::Core;

const N: usize = 1024;

// `hits` and `misses` have the same body on purpose: callgrind's
// `--toggle-collect` tells the two measurements apart by function name, and
// `#[inline(never)]` keeps each a function of its own.
#[inline(never)]
fn hits(c: &mut Core, names: &[Vec<u8>]) -> usize {
    let mut k = 0;
    for n in names {
        k ^= c.install(std::hint::black_box(n)).unwrap().addr();
    }
    k
}

#[inline(never)]
fn misses(c: &mut Core, names: &[Vec<u8>]) -> usize {
    let mut k = 0;
    for n in names {
        k ^= c.install(std::hint::black_box(n)).unwrap().addr();
    }
    k
}

fn main() {
    // names shaped like base's (8 to 16 bytes, a common prefix)
    let make = |tag: &str| (0..N).map(|i| format!("{tag}.name_{i}").into_bytes()).collect::<Vec<_>>();
    let mut c = Core::new();
    let old = make("as");
    for n in &old {
        c.install(n).unwrap();
    }
    let new = make("is");
    let a = misses(&mut c, &new);
    let b = hits(&mut c, &old);
    println!("{}", a ^ b);
}
