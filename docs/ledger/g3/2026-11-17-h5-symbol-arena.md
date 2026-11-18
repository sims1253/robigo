## 2026-11-17: H5, step 3: the process's symbol arena, a PRINTNAME per isolate

- Why (m2m3-plan §H5, docs/g3.md §5.3, L6; design note
  `~/.cache/robigo-research/h5-design.md`): symbols are process-shared,
  one per name, with global ids. The foreman approved §4 (B): a symbol's
  PRINTNAME is each isolate's own, from its CHARSXP cache, so GNU R's one
  CHARSXP per string holds in every isolate with no race. The conditions:
  - C's PRINTNAME goes through the same table.
  - A two-isolate `match` test under contention.
  - Measure the as.character-, paste- and deparse-heavy rows.
- What:
  - `crates/robigo-heap/src/arena.rs`: one arena per process.
    - A `pthread_mutex_t` guards it, with `pthread_atfork` handlers
      (prepare locks, parent and child unlock), so a child forked while
      another thread interns can intern too.
    - Its segments are 64 KiB-aligned, carry the shared pseudo header
      (`heap_id` says shared, as for #832's constants), and are never freed.
    - The name table and global ids start at 2 (`R_UnboundValue`,
      `R_MissingArg` are 0, 1).
    - A symbol is complete before it is published, and nothing writes it
      after. Its flags and name bits come from `set_symbol_bits`. Its CAR
      is the arena's own CHARSXP-shaped copy of the name, its CDR
      `R_UnboundValue`, its TAG and attributes `R_NilValue`.
    - `R_RestartToken`'s markers are arena symbols too.
  - `Core` (gc.rs):
    - `install` is a hit in the heap's own table (no lock), or this heap's
      PRINTNAME (`mkChar`, or the CHARSXP `install_char` was given), then the
      symbol's count, then the arena.
    - `printnames[id]` keeps the PRINTNAME, immortal, with GNU R's
      HASHASH/HASHVALUE marks. `Core::printname` and `Core::has_symbol` read
      it.
  - Readers: `Sym::name(cx)` is the isolate's PRINTNAME, and
    `Sym::name_bytes`/`Sexp::sym_name_bytes` give the bytes.
    `Cx::printname_raw` serves raw code: serialize, `switch`'s compiler,
    constant folding. Every caller was converted (rtbuiltins' `c`/`unlist`
    helpers take `cx`). A debug assertion in `Sexp::car` refuses a symbol,
    and found the raw readers.
  - C (`api.c`): `PRINTNAME` and the last CAR of `CAR`, `CAAR`, `CADR`, ...
    `CAD5R` of a symbol call `robigo_rtapi_printname`, the isolate's.
  - Entry (the foreman's decision after Sol r3): an isolate takes a symbol
    it did not make only by `install` of its name or `Cx::adopt_symbol`
    (`Core::adopt_symbol`), which gives it this isolate's PRINTNAME and
    table entry, fallible (`NeedGc`) at the boundary that calls it. Another
    isolate's marker (`R_RestartToken`, named "") gets the shared "" under
    its own id. A symbol's identity needs no adoption: `owns` passes it
    (step 2's form), and bindings, tags and pointer comparisons work in
    every isolate. Its name does: `Core::printname` reads the table and,
    for a symbol the isolate never took, refuses with a cold panic naming
    the id (no lazy install, no allocation, never a null slot). A base
    binding of it is refused the same way (`Loc::store`'s base arm, which
    `SET_SYMVALUE`, base definitions and active bindings all reach), since
    base's listing walks the isolate's own symbols.
- Accounting (#832's deferred notes):
  - `nodes_in_use` (GNU R's count, which `gc()` reports) is the heap's live
    objects plus the five shared constants plus the arena symbols this heap
    installed or made, each counted as if allocated (the poll and the count
    of a node).
  - `stats.live_objects`/`live_bytes` are this heap's memory only, as
    documented.
  - `shared::counts()`'s symbol rows use `node::size(SYMSXP)`, which is the
    same on today's layout.
  - Checked on a `gc()` table probe (`gc()` twice, then a new symbol).
    GNU R 4.6.2 with no default packages: 50,623 Ncells and 133,182 Vcells
    used. A `gc()` call costs 141 Ncells and 319 Vcells, a new symbol 18 and
    34. robigo-rt before and after this step prints the same numbers:
    8,226 and 4,687; 184 and 539; 32 and 153. The Mb columns are the same
    too. robigo's table does not equal GNU R's (it was not equal before H5);
    H5 leaves it as it was.
- Tests:
  - robigo-heap: the arena's threads interning at once (under Miri), a name
    longer than a segment, two heaps sharing a symbol with their own
    PRINTNAMEs, a fork child that interns a new and a known name while
    another thread interns. The symbol tests changed for global ids.
  - robigo-rtcore: two isolates on two threads interning 200 names at once,
    with `mkChar` first in one and the symbol first in the other. Each name
    is one symbol and one id, each PRINTNAME is the isolate's own `mkChar`
    result (`match(x, as.character(quote(foo)))` is TRUE in both), and each
    isolate has its own value (also under Miri).
    `a_symbol_is_every_isolates` replaces the per-heap isolation test: a
    symbol only B installed is A's by identity; A adopts it before reading
    its name. A name read and a `SET_SYMVALUE` without adoption are refused
    (two should-panic tests). Every pre-installed symbol and marker read
    from a second isolate (B's `R_RestartToken` refused, then adopted as
    "", not the symbol `install("")` gives). Base's listing from a second
    isolate after adopting and binding. Adoption under contention: A
    installs fresh names while B adopts, reads the PRINTNAME (B's `mkChar`)
    and collects between reads (also under Miri, without the collections
    and the refusal).
  - robigo-rtbuiltins `tests/isolates.rs`: the same with GNU R's `match`
    builtin run in B on every adopted name, before and after a collection.
  - C: `tests/rt/capi/pairlist.R` checks that `CAR(s)`, `CAAR(cons(s, nil))`,
    `CAR(CADR(list2(nil, s)))` and `mkChar(name)` are `PRINTNAME(s)`
    (recorded on GNU R 4.6.2).
  - CI's Miri step runs the arena's tests (`arena::`).
- Where a symbol can enter an isolate that did not install its name
  (2026-11-18, for the foreman's decision after Sol r3; read from the code
  at `ba4e6273d`):
  - Closed, inside one isolate. Each of these installs or registers the
    symbol in the isolate when the isolate is made or when the symbol is
    made:
    - `R_UnboundValue` and `R_MissingArg` (`Core::new` stores the shared ""
      under their ids).
    - `R_RestartToken`, one marker per isolate (`new_marker_symbol` stores
      the shared "" under its id).
    - The pre-installed `RSym` symbols and the C API's symbol shortcuts
      (`install_rsyms`; `CApi::new` takes them from the isolate's own
      install).
    - `install`, `install_char`, unserialize, `as.name`, the parser and C's
      `install`: all go through `Core::install_named`.
    - The base listing (`for_each_installed`) walks the heap's own table.
      No code maps an id back to a symbol.
    - No `static`, `OnceLock` or `thread_local` in the rt crates holds a
      `Sexp`. `Generic` and the `Bound`/`Wide` tables hold names only.
  - Decided (foreman, 2026-11-18): explicit adoption with a defined
    refusal (above). No R program reaches path 1. Paths 2 and 3 adopt at
    their entry.
  - Open:
    1. Safe Rust with two isolates. The `'g` brand is not generative, so a
       `Sym` or `Ref` from B's `Cx` type-checks with A's. `owns` passes every
       shared object, so any API that takes a handle accepts B's symbol:
       `cons`/`lcons` tags, `define_var`, `set_symvalue`, `var_binding`,
       `Sym::name(ca)` directly. The symbol can be any symbol, and it can
       be stored deep inside A's objects before anything reads its name.
       `tests/env.rs::a_symbol_is_every_isolates` does this on purpose.
       Sol r3's S1 (B's marker in A), S2 (B's symbol bound in A's base) and
       S3 (an allocation refusal at the first read) all enter here.
    2. Planned: docs/core.md §10 A.2's frozen segment (zero-copy sharing
       between isolates). A frozen object carries B's symbols into A's R
       code. It can be closed at its own entry: attaching the segment
       registers the symbols it holds.
    3. C statics (`static SEXP s = install("x")`) in a package's `.so`
       hand one isolate's symbol to another isolate's call. A.2 closes this
       by policy: a package with C statics that hold `SEXP`s runs in only
       one isolate unless it opts in. Note that this C idiom is common.
       Through the C API a refusal is a panic crossing `extern "C"`, so it
       aborts: acceptable for a violated invariant, and this idiom is the
       one realistic way C gets there. The multi-isolate opt-in audit must
       look for it; such a package needs its symbols adopted per isolate
       (no code for it now).
  - Deferred design proposal (outside H; to the lead through the
    foreman): a generative brand on `Cx` (`OwnedIsolate::enter(|cx| ..)`
    with an invariant `'id`). Handles of another isolate would then fail
    to compile, which closes path 1 statically, and `check_owns` could be
    debug-only in release. Every owner check on the hot paths (`scope`,
    frame binding, dispatch) would be gone, which should show as fewer
    instructions on the calls rows. The cost is an API change across
    rtcore and every crate that holds a `Cx`.
- Verified: (pending)
- Measured: (pending)
- Review: sol-loop, 2 rounds with gpt-6.1-sol on high, approved. Round 1
  found the composite C accessors and the uninstalled symbol (both fixed
  above).
