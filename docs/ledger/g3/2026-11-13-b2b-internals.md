## 2026-11-13: B2b: the base subset's internals (coerce, bind, seq, attrib readers, unique, apply, options, summary, context, sprintf, eval)

- What: m1-plan §B B2, the B2b half; docs/g3.md §7 (E7), §10 ("ported as
  functions"). Every builtin is a port of GNU R 4.6.2's C function, marked
  `// R: file.c:function`, its messages verbatim. Base: main `faa18748`
  (C1 #658) with B3's open `g3/b3-print` (#659) merged in, because the
  cases print lists and calls; V1b's `g3/v1b-vm` (#660) is **not** merged:
  its `call.rs` conflicts with C1 on main, which is V1b's to resolve.
- New in `robigo-rtbuiltins`, one module per C file:
  - `coerce/vector.rs` (coerce.c): `coerceVector` in full (every atomic
    pair with `CoercionWarning` in GNU R's order, lists, pairlists, calls,
    symbols, `PairToVectorList`, `VectorToPairList`), `asInteger`,
    `asReal`, `asComplex`; `coerce/builtins.rs`: `as.character`,
    `as.integer`, `as.double`, `as.numeric`, `as.complex`, `as.logical`,
    `as.raw`, `.Internal(as.vector)`, `typeof`, `is.vector`, `is.na`,
    `is.nan`, `is.finite`, `is.infinite`, `anyNA`, `storage.mode<-`,
    `do.call`.
  - `bind.rs` (bind.c): `c`, `unlist` (answer type, the `*Answer` fills,
    `NewExtractNames`); `apply.rs` (apply.c): `lapply`, `vapply`,
    `islistfactor`; `seq.rs` (seq.c): `rep`, `rep_len`, `seq_len`,
    `seq_along`, `xlengthgets`; `builtin.rs`: `vector`, `length`,
    `environment`, `nargs`.
  - `attrib.rs` (attrib.c, objects.c): the readers `attributes`, `attr`,
    `class`, `oldClass`, `dim`, `dimnames`, `inherits`, `getAttrib`.
  - `unique.rs` (unique.c): `match` (`match5`), `duplicated`, `unique`,
    `anyDuplicated`, `pmatch`, `match.call`; `identical.rs`.
  - `envir.rs` additions: `new.env`, `remove`, `as.environment`,
    `...length`, `...elt`, `...names`; `context.rs` (context.c):
    `sys.call`, `sys.function`, `sys.parent`, `sys.frame`, `parent.frame`.
  - `options.rs`, `errors.rs` (`gettext`, `ngettext`), `sysutils.rs`
    (`interactive`, `Sys.getenv`, `l10n_info`), `summary.rs` (`sum`, `min`,
    `max`, `all`, `any`, `pmin`, `pmax`), `math.rs` (`sqrt`, `abs`),
    `character.rs` additions (`substr`, `startsWith`, `endsWith`),
    `sprintf.rs`.
  - `util.rs`: `match_args` (GNU R's `matchArgs_NR` for builtins that match
    their own arguments) and `eval_args_keep_missing`
    (`evalListKeepMissing`).
- New in `robigo-rt`: `memory.rs` (`gc`, `reg.finalizer` over `fin.rs`),
  `do_eval` in `eval.rs` (a `CTXT_RETURN` record, as GNU R), `.Primitive`;
  special `.Internal`s (`lapply`, `vapply`) and the registry's specials
  (`rep`) are now callable (`call_special_args`).
- New elsewhere: `robigo-text::strtod` (`R_strtod5`, moved from the old
  runtime's port) and `robigo-text::printf` (one glibc conversion, for
  `sprintf`); `robigo-rtcore`: a `deparse` hook (`deparse1line_ex`, under
  `&Cx`, installed by rt from B3's deparse) for `as.character` of lists and
  calls, `Cx::heap_report`, `Cx::reset_maxused`; `robigo-heap`:
  `R_N_maxused`/`R_V_maxused` kept at each collection's start, as
  `R_gc_internal` does.
- Decisions, each where g3.md, the plan or the foreman left a choice:
  - **String keys (E7, §7).** `match`, `unique`, `duplicated` hash the
    CHARSXP's address unless an element carries a latin1 or UTF-8 mark;
    then the bytes of the UTF-8 translation. Without a mark, GNU R's
    `sequal` reduces to address equality (every CHARSXP is cached and their
    `ENC_KNOWN` agree), so the comparison is one word. The table is built
    and read inside one `&Cx` interval, so no collection moves a key under
    it (`ROBIGO_GC_MOVE=1` safe).
  - **`.Options` is rebound, not edited in place**: an assignment makes
    the updated list and rebinds `.Options` in base (robigo has no
    pairlist writer outside V1b's facade yet). It differs from GNU R only
    for code that holds the old list object. The initial list is
    `InitOptions`' entries and `Common.R`'s installation-independent ones;
    `Rprofile.unix`'s path-derived options (`pager`, `papersize`, ...) are
    not set. `options()` with no arguments sorts by name with
    `orderVector1`, which needs ICU collation (B1's decision): it says
    "not implemented yet". Options GNU R reads that robigo-rt does not yet
    (`width`, `digits`, `warn`, `expressions`, ...) are refused after GNU
    R's own checks of the value, rather than accepted without effect.
  - **`gettext` with a domain** returns new native CHARSXPs (GNU R's
    `mkChar(translateChar(...))` of an identity translation under
    `LANGUAGE=en`); without one (no namespace on the frame chain) the
    argument itself, as GNU R.
  - **`lapply`/`vapply`** evaluate `FUN(X[[i]], ...)` through the eval hook
    with `i` written in place in the closure's frame (re-made when FUN
    captures it), as GNU R; `R_forceAndCall` would also force `X[[i]]`
    before a callee that never reads it: not observable without a `[[`
    method with effects.
  - **`sys.function`** returns the record's closure, GNU R a `duplicate`
    (same value; a different object only to code that edits it in place;
    the facade's deep duplicate is V1b's).
- Not here, each an error that says so (AGENTS rule 6): the attribute
  *setters* (`attr<-`, `attributes<-`, `class<-`, `oldClass<-`,
  `environment<-`) wait for V1b's writers (`install_attrib`, `set_tag`,
  `duplicate` in `edit.rs`); `matrix`, `which`, `rawToChar`, `formals`,
  `iconv`, `rapply`; `sprintf`'s `%a`; S4 and package-ALTREP inputs;
  `mtfrm` and Date matching; `options(<name> = )` for the options GNU R
  mirrors in C state (`scipen`, `OutDec`, ...). Conditions (`stop`,
  `.addCondHands`, ...) are S1's, `.Call`/`dyn.load` A1's.
- Proof:
  - tests/rt: 37 new cases (coerce, bind, seq, attrib, unique, options,
    summary, context, dots, builtins), recorded on the 4.6.2 oracle image
    (`scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh`), all passing
    and ratcheted; the rt column is 91/96 (the 5 failing are N1's `env/*`
    and C1's `args/binding-types.R`, which print through `UseMethod`).
    Cases that need V1b (`[[`, `$`, complex assignment) or closures outside
    the loader's bound (`integer`, `assign`, `options`) call the
    `.Internal` or another bound closure instead.
  - tests/r sweep (`ROBIGO_CONF_SWEEP=1`): 88/703, 6 newly passing,
    ratcheted (`env/env-profile`, `vec/err/err-{any,match,rep-times,
    seq-len}`, `vec/raw-compare`).
  - Unit tests: `robigo-text` printf (glibc's outputs) and the crates'
    existing suites pass.
- E7 measurement (§15.3). Machine: AMD Ryzen 9 5900X, 47 GiB, WSL2, shared
  with other agents (instructions are unaffected). Binary: `robigo-rt`
  release from `e8bb0551` (sha256 `50a1fafb…`), run by
  `scripts/idiom-vs-r.py --rows ROWS --cycles --no-r --robigo BIN`. The
  rows file `benchmarks/rt/calls-rows.toml` builds `q` and `dup` with `[`
  (V1b); the measured rows use setups that build the same strings with
  `paste0` and `rep`, and the rows' own bodies:

  | row | bar (today's robigo) | robigo-rt instr | robigo-rt est. cycles |
  |---|---:|---:|---:|
  | e7-paste `paste(q, i)` | 49,821 | 125,760 | 155,883 |
  | e7-match `match(q, keys)` | 114,413 | 106,748 | 121,900 |
  | e7-unique `.Internal(unique(dup, FALSE, FALSE, NA))` | 181,394 | 111,101 | 115,040 |

  - e7-match is under its bar (from 117,867 before the address-equality
    fast path and the probe masks).
  - e7-unique is measured at the `.Internal`: the row's `unique(dup)` is a
    generic (`UseMethod`, V1b/S1). The internal leaves 70k instructions for
    the dispatch to `unique.default` and its closure call; re-measure once
    V1b lands.
  - **e7-paste regresses: 2.5x today's robigo** (GNU R 4.6.2: 177,406).
    Cause, from callgrind (per iteration, 100 elements): interning each
    result CHARSXP in the heap's global cache (`mk_char`: the embedded-NUL
    scan, the hash, the probe, the allocation) costs about 310
    instructions per string; `paste_all`'s per-piece encoding decisions
    and translations about 400 (two passes over the pieces per element,
    as `do_paste` makes them). Today's robigo does not intern its strings.
    Removing the per-element copy (paste now interns from its reused
    buffer) and the ASCII scan of native strings in `translate_char` took
    it from 162,060 to 125,760. What remains is §7's interning cost (D10
    measures it) and B1's port; recorded here for the lead.
- Review: Sol (gpt-6.1-sol, high), 2 rounds, approved. Round 1 found a
  parsed `"NaN"` cast to 0 by `as.integer` (C's cast gives NA), `eval` in
  a list with two equal names finding the second, `incomparables`'
  coercion warnings dropped by `match`/`duplicated`, and `options(width =,
  digits =)` accepted without effect; each fixed with a case recorded on
  4.6.2.
- Under `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=1` and `ROBIGO_MATERIALIZE=1`
  every new case gives GNU R's output (the release binary run on each);
  `scripts/rt-ci.py` passes.
- Open, for the PR's reviewers: merge V1b, then port the setters on its
  writers, swap `.Options` to in-place edits, re-measure e7-unique on the
  row as written, and run the tests that need `[[`/`$` (`gettext`,
  `lapply`, `stop`'s `.makeMessage`, `rm`).
