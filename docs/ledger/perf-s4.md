# perf-s4: S4 (methods) at peak against the rivals

Metric: `scripts/rivals.py run --filter 'robigo/s4-'` peak (fastest warmed-up
iteration) against GNU R 4.5.0, Ř and FastR; gate: `scripts/perf.py compare`
of instruction counts (no benchmark more than 0.5% slower) and conformance in
all three tiers.

## 2026-11-04: where the time goes (baseline, main bf7332ee)

- Peak on this VM (s): robigo / GNU R / Ř / FastR
  - s4-generic 0.0089 / 0.0073 / 0.0275 / 0.0232
  - s4-new 0.0135 / 0.0091 / 0.0101 / 0.0048
  - s4-portfolio 0.0103 / 0.0063 / 0.172 / 0.0166
  - s4-slot-read 0.00098 / 0.00082 / 0.0091 / 0.0057
  - s4-slot-write 0.0037 / 0.0029 / 0.013 / 0.0029
  - s4-valid 0.0136 / 0.0109 / 0.021 / 0.0092
- A peak iteration re-runs the whole file, `setClass` included: `setClass`
  is 0.75-0.9 ms in robigo against 0.45-0.55 ms in GNU R, so it decides
  s4-slot-read (the 10,000 slot reads take 0.35 ms in both) and a third of
  s4-slot-write and s4-portfolio. s4-new is 500 `new()` (27 us each, GNU R
  18 us), s4-valid 1000 `validObject()` (14 vs 10 us).
- All of it is methods' R code. robigo does the same R-level work as GNU R
  (`validObject`: 21 closure calls and frames in both; `new`: 37 vs 38) and
  executes 1.13-1.18x GNU R's instructions (`validObject` 171K vs 155K,
  `new` 335K vs 282K, `setClass` 5.4M vs 4.6M), but at a lower IPC: 2.43
  against 3.08 on `validObject`, 1.67 against 1.93 on `setClass`, with about
  twice GNU R's L1i misses, 1.7x its L1d misses and (setClass) 2.6x its
  branch misses (`perf stat`). The time is spread over the call machinery
  (frames, promises, contexts, `Vars` flushes); no builtin stands out.
- Calls with named arguments (`f(a = x)`, everywhere in methods) took the
  general `match_args` path: 0.32 us against 0.21 us positional, GNU R 0.22.

## 2026-11-04: Value::clone without a jump table (kept)

- why: the derived `Clone` of `Value` dispatched on the variant through a
  jump table whose arms were all "increment the `Rc`'s count" (the count is
  first in every `Rc`); the indirect branch mispredicts as values of
  different types alternate. `drop_glue::<Value>` and `Value::clone` were 6%
  and 5.6% of setClass's branch misses.
- what: a manual `Clone` that increments the payload's count in each arm
  (`mem::forget(Rc::clone(r))`) and copies the bits once (`ptr::read`); the
  arms compile to one block, so the jump goes to the same target for every
  reference-counted type.
- result: setClass loop -20% branch misses, -7.7% cycles (min of 5);
  validObject -2.5%; instruction counts unchanged (-0.04..+0.04%).
- not done: the same for drop glue needs control over the enum's drop,
  which Rust only gives through `ManuallyDrop` payloads (or reading the
  `Rc` layout).

## 2026-11-04: frame presizing and a lazy hash index (rejected)

- why: `validObject`'s frame grows to ~18 bindings one at a time: 9
  `SmallVec` regrowths and a 13-entry hash index per call (~6% of its
  instructions).
- presizing the call frame to formals + locals the compiled body assigns:
  -0.02% instructions on `validObject`, +0.1-0.3% on everything else (the
  per-call capacity lookup); no cycle gain.
- no index for call frames up to 32 bindings: `validObject` -8% cycles, but
  +0.3% startup instructions (bootstrap code looks up large frames from the
  AST interpreter) and Bounce_nonames +0.77%; built lazily after 8 scans of
  a large frame: +0.3-0.8% everywhere (the extra test in `Frame::position`).
  Both fail the instruction gate.
