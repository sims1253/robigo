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
- **Constants pool.** `Comp::pool`, keyed by object address, stays hashed
  and grows as constants are used, as on main. A capacity hint sized from
  the byte code's constants was tried and dropped. Sol found it unsafe three
  ways, because `mkCode` takes any constants:
  - NULL constants (their length read from a non-vector);
  - an ALTREP list (whose header holds no length);
  - a long plain list a code barely uses (up to about 1 GB reserved, and
    4 MiB even at the pool's `u16` cap).

  Two regression tests stay:
  - `tests/rt/bctrans/null-consts.R`, recorded on GNU R 4.6.2;
  - capi's `byte_code_with_altrep_constants_translates`, which aborted on a
    4.8 PB allocation with the hint as it was after round 1.
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

Main 90cab787a against d57a999e8 (this branch with that main merged, no
pool hint). Both were built into empty targets:

- `robigo-rt-main-90cab787`, sha256 `75bc4982a73aae126ba57b02eaa3c612d5d380d7570435869c926ad1016da534`;
- `robigo-rt-dt-d57a999e`, sha256 `55c79cdefc48c25a540795fe5b63455583aac3dd2296845b1172e05e68d0f131`.

`--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`, under callgrind; the rdb
boots from one base-database tree written by main's binary:

| | main | this | change |
|---|---:|---:|---:|
| rdb boot, JIT on | 95,323,807 | 92,537,358 | −2,786,449 (−2.9%) |
| rdb boot, JIT off | 50,823,514 | 49,976,004 | −847,510 (−1.7%) |
| interim boot | 15,433,265 | 15,427,289 | −5,976 |
| 155 closures translated (JIT on) | 19,140,045 | 16,569,373 | 123k → 107k each (−13%) |
| 641 promise codes (JIT on) | 2,974,943 | 2,830,858 | −4.8% |
| jit-scale_by, per compile | 9,223,211 | 9,219,732 | −0.04% |
| jit-safe, per compile | 13,833,637 | 13,827,029 | −0.05% |

- **The cost removed** is what C1's split of the translation predicted:
  the two `PosMap<usize, usize>`s' inserts and rehashes, less the tables'
  own construction. With the pool hint (dropped) it was −3.15M on main
  a4f1bb10c; the pool's own rehashing is the difference.
- **perf-rt (`scripts/rt-bars.sh`, median of two runs each):** every row
  is within ±0.05% instructions under both boots, except two:
  - superassign under the interim boot is −0.21%;
  - s3 under the rdb boot is +1.19% (16,364 → 16,559), and that is not
    this change's code. Run alone, or by `rt-bars.sh --keep`, it is 16,356
    against main's 16,364. It is +1.19% only when the row's scripts sit at
    a 19-character path, as `idiom-vs-r.py`'s `/tmp/idiom-XXXXXXXX` is:
    reproduced at `/tmp/idiom-keep0001`.
  - The +203 instructions per iteration are all glibc's allocator: `free`
    +79, `malloc` +10, and two unnamed glibc internals +96 and +18. No
    robigo function differs by more than 0.1. The path's length shifts the
    process's initial heap, and with this binary's layout malloc takes a
    slower path for the same allocations; main's layout happens not to.
    It is the same mechanism as #962's s3.
  - Files: `~/.cache/robigo-research/x-p1/s3-dt2` (low) and `s3-dt4`
    (high).
- **`stubs_for`** (interim boot): 699.7k on main, 703.5k here, +3.8k
  (+0.5%) for 766 lookups. The set's lookup is no longer inlined into
  `stubs_for`; the hand-written `Name` hash saved 1.2k of it. It is 0.02%
  of the boot; the foreman accepted it as measured.
- **Against GNU R 4.6.2:** its JIT-on start is 87.63M (x-p1/startup-ec22),
  so the rdb boot is now +5.6% above it. With the JIT off it was already
  below.
