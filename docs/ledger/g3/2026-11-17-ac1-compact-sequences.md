## 2026-11-17: AC1, robigo-rt's own compact sequences

- What: the first part of AC (design `~/.cache/robigo-research/ac-design.md`,
  approved by the foreman with its six decisions). It adds base's compact
  integer and real sequences from altclasses.c as robigo-rt's own ALTREP
  classes. `:`, `seq_len` and `seq_along` make them, as
  `R_compact_intrange` does. Base: `main` at `f49b7d80`.
  - **The classes** (`robigo-rtcore/src/altrep/own.rs`):
    - An instance is GNU R's layout: the info `c(n, n1, inc)` in data1, the
      expansion or R_NilValue in data2, the class in TAG.
    - It is marked not mutable when made (`COMPACT_INTSEQ_MUTABLE` is
      undefined), so every write duplicates it first.
    - The methods, all in Rust, are those of altclasses.c: Length, Elt
      (the expansion's element once there is one), Get_region through
      `Vector::elements`, Is_sorted, No_NA, Sum, Duplicate (a standard
      vector, attributes copied), Coerce (integer to real),
      Serialized_state (the info, expanded or not) and Unserialize (with
      its error for an increment other than 1 or -1).
    - Dataptr is `Cx::materialize`. It allocates as `allocVector` does,
      collecting first when a collection is due.
  - **The header bit:** an instance carries `extra::vector::OWN_ALTREP`
    (bit 2, `robigo-heap`'s `layout/defs.rs`), which only `own.rs`'s
    makers set. The hot accessors tell it from a package class's with
    one more bit test and read no class. H confirmed the bit free on main
    and in all four X3 layout cells (a code object takes bit 8, the h24
    cells bit 0). It is in the vector map's and the code map's
    no-overlap checks and "Free:" lists.
  - **Class identity:** the class object is a RAWSXP that holds a magic
    string and the class number, with GNU R's `list3(csym, psym, stype)` as
    its ATTRIB. Decision 1 named the class object's identity. A `Ref` has
    no isolate, so the typed views could not compare it against the
    isolate's globals; they read the content instead. Only `own.rs` makes
    such a RAWSXP, and a package class's table starts with function
    pointers.
  - **Typed views** (decision 2): `Vector<i32>` and `Vector<f64>` over an
    instance read it by element. `as_slice` returns the expansion and
    panics on an instance that is not expanded, so a missed site is loud.
    The ALTREP test is `Elt::OWN`: true only for the element types of the
    classes' base types, so string, list, logical, raw and complex views
    compile to the code they had before.
- **Access patterns** (decision 3):
  - A builtin's arguments are materialized at entry, as `INTEGER` and
    `REAL` expand them in GNU R (`call::materialize_args`).
  - The builtins GNU R writes against `XLENGTH`, the `*_ELT` accessors or
    no data at all leave them as they are (`reads_by_element`):
    - `length`, `seq_along`;
    - the summaries (`sum` takes the class's Sum; `min`, `max` and `prod`
      read by element);
    - `.Call` and `.External`;
    - the type and attribute queries;
    - `invisible`, `assign`, `identical`, the compiler's `putconst`;
    - `serialize`.
  - The subset and subassign defaults (`[`, `[[`, `[<-`, `[[<-`, in the VM,
    the specials and `bcassign`) read `x` by element (`ExtractSubset`'s
    `*_ELT`, `Duplicate`'s `Get_region`) and materialize the subscripts
    and the value.
  - `rep` reads `x` and `times` through `INTEGER`, as `rep3` and `rep4` do.
  - The `for` loop reads a sequence by region, as GNU R's
    `ITERATE_BY_REGION` does (`vm::for_prep`, `compact_prep`,
    `for_refill`). `for_prep` moves the sequence to a loop register and
    fills one buffer of at most 1024 elements from its formula. The step
    is the standard loop's code, with one bound compare that falls to a
    cold refill at the end of a chunk. 1024 is `BC_LOOP_COUNT_MASK + 1`,
    so the chunk-relative index ticks `BC_CHECK_SIGINT_LOOP` when GNU R's
    does. An expanded sequence is read in place. With `1:n` the loop
    allocates nothing that grows with `n` (`robigo-rtapi/tests/alloc.rs`).
- **Sites converted** (bar 6). Each of these failed or refused before:
  - `rep` and `rep_len`;
  - `vapply`, which refused any ALTREP result of FUN and now refuses only a
    package class's;
  - `unlist` and `c`: bind.c reads nested elements through `INTEGER`, so
    `Cx::materialize_nested` expands them, to depth 1 without
    `recursive`;
  - `identical`, and `unique`, `duplicated` and `match` on lists (by
    element);
  - `anyNA` (by element, as `ITERATE_BY_REGION`);
  - `paste`, `cat` and `file.path` (`list(...)`'s elements);
  - `attributes<-` and `dimnames<-` (their list's components);
  - `get_attrib`'s `row.names` (the ALTREP test now comes first);
  - the summaries.
- **The C API** (decision 5, robigo-rtapi):
  - `DATAPTR`, `DATAPTR_RO`, `INTEGER`, `REAL`, ... materialize one of
    robigo's sequences.
  - `DATAPTR_OR_NULL`, `INTEGER_ELT` and `REAL_ELT` read it as it is.
  - `LENGTH`, `XLENGTH` and `length` give its length.
  - `CHKVEC` and `CHECK_BOUNDS_ELT` give GNU R's errors: `i == XLENGTH(x)`
    passes GNU R's test and gets the class's formula.
  - A package class's Dataptr, Elt and Length are refused from C as before
    (M5).
  - The expansion C holds is pinned under `ROBIGO_GC_MOVE=1`.
- **Write paths** (bar 7): every write goes through `edit.rs`, which
  always duplicates an instance (`make_unique`) and expands the value it
  reads (`edit_pair`). `Unique::new` debug-asserts that its target is not
  ALTREP. `tests/rt/seq/compact-writes.R` writes into `x <- 1:10` through
  each path: `[<-`, `[[<-`, `attr<-`, `names<-`, `dim<-`, `class<-`,
  `attributes<-`, `is.na<-`, `length<-`, `storage.mode<-` and `$<-`, at
  top level and in compiled loops. `@<-` has no row in robigo-rt yet, and
  in GNU R it needs the methods package. C's write through `INTEGER(x)`
  is in `tests/rt/capi/compact-seq.R`.
- **serialize** (bar 1, bar 3):
  - Version 3 writes an `ALTREP_SXP` (the class's info, its state, its
    attributes) before the persistent names and references, as GNU R
    orders it.
  - Version 2 writes the standard vector.
  - The reader rebuilds the sequence, with the attributes.
  - `coerceVector` calls the class's Coerce, so `as.numeric(1:10)` is
    GNU R's compact real sequence and serializes as one. Under `&Cx` it is
    made expanded: its callers read it through `REAL`.
  - The INTERIM(AC) exception and its counter are gone: all 2,440 records
    of base (1,189), compiler (297) and tools (954) serialize back to GNU
    R's round trip.
- **The JIT's folds:** robigo-rt's fold helper put every folded value in a
  list before its `checkConst` test. That expanded `1:4000` in a `for`
  loop's header. It now tests the length (`maxConstSize`, 10) first, as
  cmp.R's `checkConst` does.
- **E9 (`new_compact`):** the class object was read as a raw `Sexp` before
  `cx.build`. A retry after a moving collection then gave the instance a
  stale TAG. It is now read inside the closure.
- **Known divergences:** each is visible only through C's
  `DATAPTR_OR_NULL` and `.Internal(inspect())`, never in a value.
  - About 28 builtins still expand a sequence where GNU R reads it by
    element: `print`, `sort`, `order`, `unique`, `duplicated`,
    `as.numeric`, `as.character`, `as.integer`, `paste`, `is.na`, `match`,
    `%in%`, `sqrt`, `nchar`, `is.unsorted`, `xtfrm`, `rank`, `factor`,
    `as.list`, `as.vector`, `&`, `xor`, `is.finite`, `is.nan`,
    `cumprod`, `list`, ... (measured against GNU R 4.6.2 with a probe
    that reads `DATAPTR_OR_NULL`). That is AC2's audit.
  - `list` materializes its arguments. It only stores them in GNU R, but
    the builtins that take a list of values do not all read by element
    yet.
  - `identical` reads by element where GNU R's `INTEGER_RO` expands two
    vectors of the same type and length.
  - The reader expands a nested sequence.
  - A write that C makes into an expansion while a loop is reading the
    sequence is seen from the next chunk; GNU R's `INTEGER_ELT` reads it
    at once.
  - `min`, `max`, `prod` and `sum` of several arguments read an
    unexpanded sequence into a temporary Rust vector of its length
    (`Vector::elements`), where GNU R scans bounded regions: memory, not
    values (Sol's S1; AC2).
  - `row.names` is still made a standard vector.
- **`.Internal(inspect())`** is not ported to robigo-rt. Its output has
  addresses and GC fields, so a recorded case would need them filtered.
  Bar 3's inspect part is covered instead through `DATAPTR_OR_NULL` from
  C, which reads the same state as inspect's "(compact)" or "(expanded)".
- Proof:
  - Recorded on 4.6.2 and ratcheted:
    - `tests/rt/capi/compact-seq.R`: the C API; byte-identical with GNU
      R.
    - `tests/rt/serialize-closures/compact.R`: versions 2 and 3, xdr, binary and
      ascii; nested; shared; an expanded sequence; attributes; a
      hand-built `ALTREP_SXP` with attributes. It is in
      `serialize-closures`, outside base-subset's traced areas, as
      `frames.R` is: it serializes a closure, and the tracer rewrites the
      body of every closure in the global environment, so the traced run's
      bytes differ from the untraced run's.
    - `tests/rt/seq/compact-writes.R` and `tests/rt/seq/compact-nested.R`.
  - `robigo-rtapi/tests/alloc.rs`: the same allocation count for n = 2000
    and n = 10^6 (both past one chunk), in AST, byte code and JIT.
  - `robigo-rtcore/tests/compact.rs`, which runs under Miri.
  - E9 under `ROBIGO_GC_MOVE=1` at every allocation point of the four
    cases: 48,439 points, clean.
  - Corpora on robigo-rt against main's binary: fastr gains 3 and loses
    none; gnur-tests is equal. The three are ratcheted (`passing-rt`):
    `builtins/TestBuiltin_serialize/` `03e9e5aa894e97cd`
    (`serialize(1L:10L, NULL, version = 3)`), `a47a59a0134ad4ce`
    (`1:10`) and `6260e145e42c04ad` (`1:2147483649`, a compact real
    sequence of a few bytes that main wrote as 2^31 doubles; A1's memory
    sweep found it).
- **Performance** (callgrind instructions; the gate is 0.5% per row):
  - Against main at `f49b7d80`, `perf.py rt-bars`: plain −1.00%, defaults
    −0.70%, named −0.01%, env-fn +0.49%, superassign +0.16%, s3 +0.50%,
    fib −0.17%, binarytrees +0.38%, e7-paste +0.21%, e7-match −0.11%,
    e7-unique +0.19%. AC1 holds #798's `vm::num` change; the foreman
    ordered #793, AC1, then #798.
  - churn's peak RSS with A1's in-loop `gc()` (#793): 78,448 KiB against
    main's 78,524.
  - The loop's refill costs 1.1 instructions per iteration, against 2.5
    for main's `intrange` building `1:n`.
  - The rest is the plain-vector fast path's cost (bar 8). The ALTREP arms
    of the shared accessors (`len`, `as_vector`, `get`) change register
    allocation in `vm::run` (31,486 bytes and 1,337 stack operands,
    against 30,753 and 1,244 for main + #798) and the compiler's inlining
    elsewhere (s3's dispatch functions, which AC1 does not touch). With
    no loop code in `run` at all, env-fn is still +0.67% against main +
    #798, so a separate loop op would not help; moving the arms out of
    line made every row worse. The per-function table is in the PR.
