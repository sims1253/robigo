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

## 2026-11-04: measured, not changed

- PGO (scripts/pgo-build.sh's training, this branch): cycles -13% on
  `validObject`, -8% on `setClass`, -14% on `new`, -15% on generic
  dispatch (min of 3, `perf stat`); instructions -11..-15%. The nightly
  binary is built that way; `rivals.py` measures `scripts/build.sh`'s,
  which is not. Most of the IPC gap to GNU R above is code layout.
- Smaller inline buffers in `vm::run_pre` (registers 32 -> 16, slots
  24 -> 12, guard marks 128 -> 32; the frame 2.6 KB -> 1.3 KB): no cycle
  change outside noise, +0.1-0.3% instructions. Rejected.
- Call frames indexed only past 20 bindings: no cycle change (the -8%
  measured earlier with 32 did not repeat; cycles on this VM vary by +-5%
  between runs), +0.1% startup instructions. Rejected.
- `try(x, silent = TRUE)` is 46.7K instructions (GNU R 42.4K) and 28% of
  `validObject`. 10% of it is guard checking: `tryCatch`'s helpers are
  closures defined in its frame, so every call is a new chain of frames
  and `guard_ok_slow` walks it for each guard (17 walks, 66
  `local_may_bind_fun` per `try`). GNU R decides this at compile time for
  namespace code. Left to perf-vm (guards).
- The loop JIT has no `SlotRead`: s4-slot-read's 10,000 `x@x` run in the
  VM at 427 instructions each (35 ns, GNU R 34 ns). Native code would need
  `Dyn` variables in loops (only whole-closure native code has them).
- Per call, robigo's machinery is not the instruction problem:
  `h3(x)` 3,123 instructions vs GNU R 3,769, generic dispatch 10.7K vs
  12.0K; the same R-level work (21 frames per `validObject`, 663 per
  `setClass` in both) runs at lower IPC.

## 2026-11-04: `function` compiled to MakeClosure (kept)

- why: compiled code evaluated `function(x) ...` as a call of the special
  (`CallSpecial`: flush, the call's arguments copied, `do_function` looking
  its formals up in a thread-local table): 1,180 instructions per closure
  against GNU R's 364 (MAKECLOSURE). methods makes several per operation
  (`validObject`'s `anyStrings`, `tryCatch`'s three helpers, `try`'s
  handler).
- what: `Op::MakeClosure`, its formals converted once at compile time
  (`ClosureTemplate`), no flush (making a closure runs no R code; the
  closure reads the frame only from a call, which flushes), the `function`
  binding guarded as before; `do_function` and the op share `make_closure`.
- result: 1,180 -> 488 instructions per closure; `validObject` 168.2K ->
  166.0K. `perf.py compare`: 0 faster, 1 slower: `convolution_v` +0.61%,
  all of it `memset` in the allocator (+2.9M: a 2.7 MB `matrix(0, ...)`
  lands on a reused block that must be zeroed instead of fresh pages, as
  the allocation sequence changed); its R code makes no closures in the
  loop.
- verified: `tests/r/basics/compiled-closure-creation.R` in all tiers.
- gap (interpreter tier only, so no `tests/r` script, which must agree
  across tiers): `m <- function() { for (i in 1) local(2); 1 }; m()` fails
  with "attempt to apply non-function" under `ROBIGO_COMPILE=0`; the VM and
  GNU R return 1.

## 2026-11-04: named arguments and `...` without match_args (kept)

- why: a call with a tagged argument or to a closure with `...` took the
  general path (`call_closure_ex` -> `match_args`, three passes, a
  `Vec<Option<Value>>`, partial matching by name); methods' code calls
  almost everything that way (`getClassDef(classi, where = where)`,
  `new(Class, ...)`, `initialize(value, ...)`, `try(expr, silent = TRUE)`).
  `h3(a = x)` cost 3,786 instructions and 0.32 us against 3,123 and 0.21 us
  positional.
- what: `interp::bind_exact`, matchArgs_NR for the case where partial
  matching has nothing to do: tags name formals exactly, a tag that names
  none goes to `...` unless it is a prefix of an unmatched formal before
  `...` (then the general path runs, as it does for every error). Compiled
  calls then take `direct_call`'s frame/context path (`vm::tagged_call`);
  `apply_closure_ex` uses it too.
- result: `perf.py compare` against main: 45 faster, 0 slower; s4-new
  -1.0%, s4-valid -0.9%, s4-portfolio -0.9%, the binarytrees family and
  lapply -0.6..-0.8%. `h3(a = x)` 0.37 -> 0.28 us.
- verified: `tests/r/basics/named-arg-matching.R` (exact, partial and
  positional mixes, `...` before and after formals, errors, laziness,
  sys.call/match.call/substitute/nargs, `..N`), byte-identical in all tiers.

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
