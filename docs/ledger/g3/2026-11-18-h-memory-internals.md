## 2026-11-18: memory.c's gcinfo, gc.time, gctorture2, memory.profile, mem.maxNSize; what L7 asks of their numbers

- Why: the evaluator delegate's inventory of main (m2m3-plan §A) found
  memory.c entries that robigo-rt does not have. They are H's by file.
  `mem.maxVSize` was already there; `gc(verbose = TRUE)` raised "not
  implemented" for lack of `gcinfo`'s report.
- What (each ported from GNU R 4.6.2's memory.c, with names.c's kind and
  visibility):
  - `gcinfo` (`.Internal`, eval 11): GNU R's `gc_reporting` switch. NA
    changes nothing; the value is the old state.
  - `gc(verbose)`: reporting is on for its own collection, NA included
    (`if (gc_reporting)` in C).
  - The report: `Core::report_collection`, appended at the end of each
    collection to the heap's pending text. rtcore prints it right after
    the collection returns (`Cx::flush_gc_report`, through `REprintf`'s
    path: after a safepoint, `collect`, and each `alloc_rooted`). GNU R
    prints during the collection; robigo runs nothing between the two.
  - `gc.time` (a primitive, eval 1): `check1arg`'s "on" and `asRbool`, then
    the five times. A collection is timed between `begin_collection` and
    the end of `finish_collection` (not in the generic `collect_needing`)
    by `R_getProcTime`'s rules (getrusage, to the millisecond), with
    GNU R's delta 0.
  - `gctorture2` (`.Internal`, eval 11): step and wait through
    `R_gc_torture`, `inhibit_release` checked by `asRbool` and otherwise
    unread, as in a build without PROTECTCHECK. The value is the old step
    (`gc_force_gap`).
  - `memory.profile` (`.Internal`, eval 11): `R_gc()`, then the heap's
    objects by type (`Core::census_by_type`, every allocated object plus
    the shared constants, as `nodes_in_use` counts them), 24 entries named
    by `type2str` with types 11 and 12 skipped.
  - `mem.maxNSize` (`.Internal`, eval 11): `do_maxNSize` over the heap's
    node limit (`Core::set_max_nsize`, which existed).
- What the numbers are. docs/g3.md L7 (a lead decision) says that `gc()`'s
  cell counts, trigger sizes, collection counts and levels, and
  `gcinfo(TRUE)`'s report contents equal GNU R's. robigo would reproduce
  them by a logical account of the objects GNU R would have allocated,
  plus GNU R's generation schedule. That account does not exist. Per
  value:

  | value | L7 | can robigo match exactly today? | this PR |
  |---|---|---|---|
  | `gc()` used, max used (cells and Mb) | exact | no: needs the account | robigo's heap, GNU R's units |
  | `gc()` trigger (`R_NSize`, `R_VSize`) | exact | no: `AdjustHeapSize` is ported but runs over robigo's counts | as is |
  | `gc()` limit column, `mem.maxNSize`, `mem.maxVSize` | exact (limits) | yes | exact |
  | `gcinfo` switch, `gc(verbose)` | exact | yes | exact |
  | `gcinfo` report: count, `a+b+c`, level | exact | no: robigo collects at other times, every collection full | robigo's collections, level 2, `0+0+N` |
  | `gcinfo` report: Mbytes and % | exact | no: as `gc()`'s | robigo's heap |
  | `gc.time` | not in L7 | not meaningful: machine time in GNU R too | robigo's real collections |
  | `memory.profile` counts | not in L7 (same nature as cell counts) | no: needs the account | robigo's heap by type |
  | `gctorture2` value, `gc.time` shape and errors | exact | yes | exact, oracle-recorded |

- Measured, to size the account (2026-11-18; robigo-rt debug at
  `1a9d21eb1` plus this branch; GNU R 4.6.2 on the oracle image; both
  with `R_DEFAULT_PACKAGES=NULL`; `gc()` deltas in Ncells and Vcells for
  keeping one object alive):

  | | GNU R | robigo-rt |
  |---|---|---|
  | after boot | 50,595 / 132,978 | 8,102 / 4,034 |
  | `numeric(1e5)` | 21 / 100,038 | 20 / 100,141 |
  | `.Internal(as.vector(1:1000, "list"))` | 1,001 / 1,998 | 1,003 / 1,999 |
  | `paste0("zq", 1:1000)` | 1,514 / 3,021 | 1,011 / 2,106 |
  | `pairlist(1, 2, 3)` | 40 / 71 | 41 / 263 |
  | `new.env()` | 6 / 31 | 8 / 93 |
  | a closure | 4 / 2 | 7 / 71 |
  | 10 new symbols | 36 / 29 | 271 / 1,015 |
  | the measuring call itself | 4 / 3 | 12 / 70 |

  - What a ported internal keeps alive already matches, give or take the
    measuring call: vectors are counted in GNU R's VECRECs
    (`heapsize::vec_cells`), and a list of 1,000 integers is 1,003 /
    1,999 cells against 1,001 / 1,998. `memory.profile()` deltas by type
    for `as.list(1:1000)` agree (pairlist +3, integer +1,001, list +1,
    character +1, double −1), except 11 CHARSXPs robigo makes on first
    use.
  - The rest comes from four places:
    - boot: GNU R lazy-loads base's promises and installs its symbol
      table; robigo binds its wide base lazily;
    - the first use of a base function: GNU R's first `as.list` call
      costs 59,786 Ncells and 134,612 Vcells to load and compile it;
    - the evaluator: robigo's slot frames, promise-free arguments,
      compiled IR on calls and its symbols (a symbol in robigo-rt carries
      100 VECRECs more than in GNU R);
    - each builtin's temporaries.
- Proposal, for the lead through the foreman: narrow L7 to what a
  program can depend on. Exact:
  - the table's shape and units;
  - the limits and their errors and warnings;
  - `reset` and max-used semantics;
  - the trigger rules as ported;
  - the report's format;
  - for the data a builtin makes, the cells it keeps alive. This holds
    for ported internals today, so a ratchet case can check gc() deltas
    for data objects.

  Absolute totals, triggers, collection counts and levels would then be
  robigo's own and documented as such. The full account needs four things:
  - a model of GNU R's boot (its lazy-load promises and symbol table);
  - first-use loading and JIT compilation;
  - per-call promises and argument pairlists, and the bindings of every
    frame;
  - each builtin's temporaries (`R_alloc` included).

  Its counters would sit on the evaluator's paths, which §13 measures, and
  no R program's result depends on these numbers. Until the lead decides,
  the numbers stay robigo's heap in GNU R's shape. That was `gc()`'s
  behavior on main already, and this PR does not change it.
- Note: the 10-symbols row says a new symbol costs robigo-rt about 27
  nodes and 100 VECRECs, against 3.6 and 2.9 in GNU R. H5 (#851) changes
  how symbols are counted; re-measure after it lands.
- Tests:
  - `tests/rt/memory/internals.R`, recorded on GNU R 4.6.2: `gcinfo` (NA
    and both states), `gc.time` (shape, `check1arg`, NA, length 2),
    `gctorture2` (old step, NA `inhibit_release`), `memory.profile`
    (names, type, and deltas that hold in both), and `mem.maxNSize`
    (Inf, a limit below usage, a negative value, 1e12, back to Inf).
  - Heap unit tests: the report's three lines, only while reporting is on;
    the census counts live objects by type, the shared constants
    included; `gc.time` counts nothing while off.
- Verified: (pending)
- Measured (perf): (pending)
