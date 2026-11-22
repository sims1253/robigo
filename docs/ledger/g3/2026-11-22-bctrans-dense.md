# The byte-code translator's maps as dense tables (2026-11-22)

X/P1, the startup plan's next item after libdeflate (#968). The translator
(`compile/bctrans.rs`) is C1's; C1 reviewed the plan and set the edge cases
below.

## What changed

- **Position index.** `PosIndex` maps a code position to its instruction
  index. It is built once per translated code and starts at the code's first
  position, so an argument's slice of a code is sized by the slice. It has
  one entry past the last instruction's start, so a target at the code's end
  is a miss, not an out-of-bounds read. `reachable`, the labels and the jump
  patches use it in place of `reachable`'s `PosMap` and pass 2's `at_ir`
  `PosMap`.
- **One record per instruction.** `At` holds what each instruction's
  translation keeps: its label state, its IR position, pass 1's consumer
  starts, pass 2's reserved register, a complex assignment, and the plan's
  reservations. It replaces six `PosMap`s keyed by instruction index. Each
  pass makes one allocation of `dec.len() + 1` records; the extra one is for
  an `open` start at the code's end. Pass 1's records, with their
  reservations made in place (`reserve_at`) and the position index, become
  the plan. Nothing is cloned for pass 2.
- **Constants pool.** `Comp::pool`, keyed by object address, stays hashed.
  It is now sized from the byte code's constant count, so it does not rehash
  while a pass fills it.
- **`fold_vis_into_ret`'s jump targets** are a bit vector of `ins.len() + 2`
  entries. A mark past it is dropped; only positions before the end are
  asked about.
- **`PosHasher`** is for fixed-width integer keys only. A byte-string key
  is a debug-assertion failure. A type alias cannot carry a trait bound, so
  the debug-assertions shard enforces it. The unused `PosSet` alias is gone.
- **`NameSet`** is the one string-keyed set: `(Name, bool)`, where
  `Name(&'static str)` is a compiled-in name (names.c's table), never a
  string a program supplies. Its hasher takes 8 bytes per multiply, and
  `Name`'s `Hash` writes the length and then the bytes, with no terminator.
  `base::stubs_for` builds it with capacity `st.defs.len()` (#962's
  deferred low).

## Order and edges (C1's spec)

- **Iteration order.** The translator iterated one map: `Tr::plan` over
  pass 1's consumers. That is order-independent, because each
  reservation list is sorted afterwards and its consumers are distinct.
  Every other use was a keyed get or insert. So index order changes no
  output.
- **Three tests check that the failures stay the same:**
  - `a_jump_to_the_end_or_an_operand_fails_as_before`: a `GOTO` to the
    code's end position or into an operand fails in `reachable` with
    `Need::Fail`, as the hashed index's miss did. A slice's positions
    below and past it miss.
  - `a_start_at_the_codes_end_is_reserved_there`: a consumer whose operand
    starts at the instruction count is reserved there, in bounds.
  - `targets_at_and_past_the_end_leave_the_fold_as_before`: jump and loop
    targets at and past the code's end leave `fold_vis_into_ret`'s result
    as before.
- **Crosscheck.** With `ROBIGO_BC_CROSSCHECK=1` over `tests/rt/bctrans`,
  every translated code that is compared is the same as the from-source
  compile.

## Numbers

Main a4f1bb10c against d2f3e2989 (this branch with that main merged; the
later commit only changes `Name`'s hash). Both were built into empty
targets:

- `robigo-rt-main-a4f1bb10`, sha256 `ce4661d4c805cb485799f27bcd5a08ad3380af6648ec2fbf3c9c4611835599e8`;
- `robigo-rt-dt-d2f3e298`, sha256 `31b07309337cb5d7bb5256c7a8fe60e29a4b32cbee44e4ca280da762513eaf29`.

`--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`, under callgrind; the rdb
boots from one base-database tree written by main's binary:

| | main | this | change |
|---|---:|---:|---:|
| rdb boot, JIT on | 95,101,076 | 91,954,628 | −3,146,448 (−3.3%) |
| rdb boot, JIT off | 50,781,117 | 49,997,334 | −783,783 (−1.5%) |
| interim boot | 15,436,808 | 15,363,007 | −73,801 |
| 155 closures translated (JIT on) | 19,039,895 | 15,991,874 | 123k → 103k each (−16%) |
| 641 promise codes (JIT on) | 2,964,012 | 2,839,592 | −4.2% |
| jit-scale_by, per compile | 9,226,508 | 9,224,876 | −0.02% |
| jit-safe, per compile | 13,835,477 | 13,824,744 | −0.08% |

- **The cost removed** is what C1's split of the translation predicted:
  the two `PosMap<usize, usize>`s' inserts and rehashes (about 3.3M),
  less the tables' own construction.
- **perf-rt (`scripts/rt-bars.sh`, median of two runs each):** every row
  is within ±0.05% instructions under both boots.
- **`stubs_for`** (interim boot): 700.0k on main, 705.9k on d2f3e2989, and
  704.7k with the hand-written `Name` hash (395e84e75). That is +4.7k
  (+0.7%) for 766 lookups, about 6 instructions each: the set's lookup is
  no longer inlined into `stubs_for`. It is 0.03% of the boot; the
  foreman accepted it as measured.
- **Against GNU R 4.6.2:** its JIT-on start is 87.63M (x-p1/startup-ec22),
  so the rdb boot is now +4.9% above it, where main ec22e65ba was +15.1%.
  With the JIT off it was already below.
