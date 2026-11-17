## 2026-11-17: the address-hashing rows are measured as the median over heap shifts

The foreman's decision, a change to the gate's measurement method that Max
can reverse. e7-match is measured in `perf-rt` (`scripts/perf.py
rt-bars`) as the median over a fixed set of heap shifts, for both binaries.
The 0.5% rule against main applies to the median. Every other row is
measured as before. e7-unique was marked too, until #757 made its count
stable ("After #757" below).

### Why

Both rows hash 1,000 cached strings by address: GNU R's `cshash`
(`unique.c:205`), which robigo-rt ports
(`crates/robigo-rtbuiltins/src/unique.rs:316`). Any change to what is
allocated before the row runs moves which blocks the keys land in, and so
how many collide. Then the row's count moves with identical code.
- **S1's BW** fails the gate on e7-match at +4.15%. With
  `zz <- paste0("z", 1:k)` before the setup, BW's lowest count equals
  main's:
  - main over k = 0 to 5,000: 98,900 to 99,276;
  - BW over k = 300 to 5,000: 98,907 to 99,610.
- **H's diagnosis of #752's failure:**
  - e7-unique moved with glibc's malloc state, while robigo's own code
    counted the same instructions;
  - e7-match moved with the CHARSXP addresses, which 17 extra boot closures
    shifted.

  H ports `HashTableSetup` faithfully, with the table as an INTSXP on
  robigo's heap.
- **GNU R moves the same way.** A0 measured it
  (`2026-11-16-a0-oracle-gap.md`, "The hashing rows"):
  - the oracle build's e7-unique drops 5.0% once 50 unrelated strings come
    first (170,397 to 161,939);
  - e7-match moves by up to 1.3%.

  robigo-rt moved by at most 0.14% over A0's smaller paddings (0 to 333
  strings), which the 5,000-string range above exceeds.

A single layout's count of these rows is therefore noise at the 0.5% level.
The decision keeps GNU R's address hash, so it is no porting deviation. The
alternative, hashing cached strings by content, which nothing observes, is
with Max and is not built.

### The method

- **The marking:** a row marked `address_sensitive = true` in
  `benchmarks/rt/calls-rows.toml`: e7-match, whose bar's comment in
  `m1-bars.toml` points at it.
- **The shifts:** `scripts/idiom-vs-r.py --rows ... --shifts K,...`
  measures each marked row once per shift.
  - Shift k puts `zz <- paste0("z", seq_len(k))` before the row's setup,
    and k = 0 is the row as written. `seq_len`, not `1:k`, so that k = 0
    adds nothing.
  - The row reports the median, and the JSON keeps every shift's count
    under `shifts`.
  - Unmarked rows run once, as before.
- **The shift set:** `perf.py`'s `ADDRESS_SHIFTS`, k ∈ {0, 300, 1000, 3000,
  5000}. `rt_instr` always passes it, so `rt-bars`, `rt-gnur` and CI's
  `perf-rt` shift robigo-rt, main's binary and GNU R's side identically.
- **The report:** the table shows the median, and a list below it gives
  each marked row's range over the shifts, next to main's. The JSON keeps
  the ranges (`shift_ranges`).
- **The gate:** `rt_verdict` compares the medians, so the 0.5% rule applies
  to them. The bars against today's robigo compare the median with today's
  committed number.

### Checks

- **`scripts/test-perf-rt-bars.py`** (25 tests) covers:
  - a real +0.6% on every shift fails the gate against main;
  - one shift alone at +4.15%, BW's case, does not;
  - `idiom-vs-r.py` measures each shifted setup of a marked row and a plain
    row once;
  - `rt_instr` passes the shift set and collects the range;
  - exactly e7-match is marked.

- **A real run:** `rt-bars --cycles` on the desktop with main `fb429442`'s
  release robigo-rt as both binaries (sha256 `237f49bb…`, GNU build ID
  `410ea739…`). It finished with 0 failures, in 57 s on 6 jobs for both
  binaries together, so the eight extra runs per binary cost little.

  | row | median | range over the shifts | main's range |
  |---|---:|---:|---:|
  | e7-match | 99,038.8 | 98,903.8–99,279.7 | 98,900.8–99,279.4 |
  | e7-unique | 126,388.4 | 126,060.6–126,569.7 | 126,062.2–126,570.0 |

  e7-match's range on main is the foreman's (98,900 to 99,276). It spans
  0.38%, and e7-unique's 0.40%, so one shift's count can swing the old
  single-layout gate by most of its 0.5%. The medians of two identical
  binaries differ by 0.01%.

### After #757: e7-unique no longer needs the median

H's #757 ported `HashTableSetup` faithfully, with the table on robigo's heap.
That removed the malloc-state dependence of e7-unique that H diagnosed.

Both rows were re-measured over eight shifts (k = 0, 50, 100, 300, 1000,
2000, 3000 and 5000), on robigo-rt release builds of main before #757
(`fb429442`) and after it (with #757):

| row | before #757: median, spread | after #757: median, spread |
|---|---:|---:|
| e7-match | 98,976.9, 0.385% | 91,955.8, 0.413% |
| e7-unique | 126,402.3, 0.554% | 123,011.9, **0.012%** |

- **e7-unique's count no longer moves** with the heap layout: 123,003 to
  123,018 over the eight shifts. Its marking is dropped, and it is measured
  once, as before.
- **e7-match still moves by 0.41%**, so it keeps the median.
- **#757 also made both faster:** e7-match by 7.1% and e7-unique by 2.7%.
