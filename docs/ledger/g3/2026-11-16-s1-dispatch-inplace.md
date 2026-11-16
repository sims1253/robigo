## 2026-11-16: S1, a method's write to its argument made in place as GNU R makes it, and the promargs filler's kind tag (#718's review notes)

- What: X/P1's two review notes on #718, in one package (the foreman,
  2026-11-16).
  1. A method's write to its argument copied where GNU R writes in place:
     `f(c(1, 2, 3))` at top level, in a compiled loop, and for a classed
     object in a compiled loop. Main had the same behaviour before #718.
  2. `generic_args` identified the lazy `promargs`' filler by its function
     address (`std::ptr::fn_addr_eq`), which Rust does not guarantee.
- GNU R's method shares the generic's promises: `dispatchMethod` hands
  `applyMethod` `cptr->promargs`, so the method's binding of `x` is the
  generic's promise. A fresh value then has one reference, the promise's
  `PRVALUE`. `EnsureLocal` finds it not shared, `x[1] <- 9` writes it in
  place, and the generic sees the write: its `on.exit(print(x))` prints
  `9 2 3`, recorded on 4.6.2. A variable's value (`v <- c(1, 2, 3); f(v)`)
  has the variable's reference too and is copied, and so is the argument
  of `NextMethod`'s method (NextMethod makes its own promises).
- robigo-rt counted the value twice, for two reasons, found with a
  watch on the value's count (a temporary backtrace at each increment):
  - the method's own binding: since #718 a forced slot copied from the
    generic's, counting the value again; on main before #718, the
    materialized list's promise;
  - `usemethod`'s root of the dispatch object. robigo-rt's roots are values
    of the byte-code node stack, which a complex assignment links
    (`crate::links`, GNU R's `INCREMENT_BCSTACK_LINKS` and
    `INCLNK_stack_commit`), so the object looked shared during the
    method's `x[1] <- 9`. GNU R holds it on the protect stack, which links
    nothing.
- Fix:
  - `args::generic_source` makes each argument slot of the generic that
    the method gets into the promise the list would hold
    (`Cx::arg_promise`; a delayed argument too, unforced). It binds that
    promise in the method's frame, with no list made. `Origin::Forced` and
    its slot copies are gone, and a delayed argument no longer falls back
    to the list.
  - `usemethod` empties its root of the object once a method is found;
    nothing reads it after that.
  - `robigo-rtcore`: `ArgSlot::Promise` and `Residues::promise_of`, so
    `generic_args` reads an argument's promise once made (a materialized
    slot, or an overwritten argument whose promise was made) without
    allocating.
- The kind tag: a record keeps its owner's tag with the lazy `promargs`
  filler. It is `Ext::fillkind`, in `Defn.h`'s padding after
  `parkedmask`, so no other offset moves; `context.c`'s layout table and
  the Rust layout check have it. `Promargs::Lazy` and
  `RawFunctionContext` carry it. `args.rs` tags its two fillers
  (`FILL_ARGS`, `FILL_GENERIC`), and `generic_args` tests the tag.
- Two costs cut on the way:
  - `end_call` took a call's residues by splitting the whole list into two
    new `Vec`s at every call with an entry. `Residues::pop_of` now takes
    them from the end (calls nest), with no lists built.
  - `generic_source` reads the generic's arguments once, rooting the
    values across `arg_promise`'s allocations, instead of reading the
    record and frame a second time.
- Measurements (WSL2 workstation, Ryzen 9 5900X, rustc 1.100.0; release
  `robigo-rt` from the private target `~/.cache/cargo-target-s1`; the M1
  bars, `scripts/rt-bars.sh` twice each against main `5bb0eabf`,
  `rt-bars-compare.py`; `data/2026-11-16-s1-dispatch-inplace/bars-compare.md`):
  s3 20,287.4 -> 20,838.4 instructions (1.0272) and 23,059.5 -> 23,724.6
  estimated cycles (1.0288), 1.10x -> 1.13x GNU R 4.6.2's 18,497.
  Allocations go from 16 to 17 objects per iteration (PROMSXP 3 -> 4: the
  shared promise). binarytrees 0.9962 and fib 0.9996 (`end_call`'s
  residues taken without lists); plain, defaults, named, env-fn and
  superassign 1.0000. By `scripts/callgrind-diff.py` the s3 row was
  21,579 with the shared promises before the two cuts.
- What exactness costs, and how to get it back: the method's arguments are
  promise objects again, as in GNU R. That is one `PROMSXP` per argument
  per dispatch (`ensure_object`, its residue, `clear_promise` at the
  generic's end): about 550 instructions on the s3 row. A slot that
  aliases the generic's argument slot until something needs the promise
  object would share it without making it. It would touch every slot path
  in `robigo-rtcore`'s frames (X/P1's): the raw lookups, `ensure_object`,
  cleanup, recycling and `missing()`. It is not in this package.
- Proof: `tests/rt/capi/dispatch-inplace.R`, recorded on 4.6.2 and
  ratcheted; production only, as `inplace-identity`, since
  `ROBIGO_NO_INPLACE=1` copies at every write. With the native observer
  of in-place writes (`support/identity.c`), a method's `x[1] <- 9` is
  made in place for a fresh value at top level, in compiled loops and for
  a classed object, and copied for a variable's value and through
  NextMethod. The generic's `on.exit` prints the method's write.
  `tests/s3.rs`: a delayed argument is shared without the list, and a
  generic that overwrote an argument makes the list, with and without
  `ROBIGO_MATERIALIZE`. `robigo-rtcore`'s tests cover `promargs_filler`'s
  kind and the layout check. All `tests/rt/s3`, `tests/rt/conditions` and
  `capi` in-place cases pass. `scripts/rt-ci.py` passes every step:
  conformance 285/290 in every mode, with gctorture, `ROBIGO_GC_MOVE`
  with gctorture, `ROBIGO_FRAME_QUARANTINE`, the small vector limit and
  the injected allocation failures. `ROBIGO_NO_INPLACE` (283/288) and the
  collector modes (284/289) run a case or two fewer. Miri on
  `robigo-rtcore` passes; `scripts/lint.sh` is clean.
