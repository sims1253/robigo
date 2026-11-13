## 2026-11-13: N1b, slot frames (`robigo-heap`, `robigo-rtcore`)

- What: every unhashed environment (closure frames, `new.env(hash =
  FALSE)`, `R_NewEnv(.., FALSE, ..)`) is now a slot frame (docs/g3.md §5.1;
  the council's Fable round 2 §3.1-3.2; m1-plan N1b). It replaces N1a's
  GNU R pairlist behind `env/frame/`, the interface N1a left for it; the
  `envir.c` entry points (N1a) run on it unchanged. New: the word map
  (`robigo_heap::slots`, re-exported as rtcore's `env::frame::layout`), the
  heap's `Core::alloc_slot_frame` and trace arm, the frame operations
  (`env/frame/mod.rs`), and `env/slotapi.rs` for the evaluator and the C
  API: `new_shape`, `new_closure_frame` with `SlotInit`, `define_scalar`,
  `frame_snapshot`, `set_frame`, `find_var_loc_in_frame`, `var_loc_value`,
  `set_var_loc_value`, `set_var_super` with `SuperAssignCache`,
  `env_has_no_special_symbols`, `set_no_special_symbols`.
- The word map (decided here; g3.md calls Fable's indicative), under §5's
  three constraints:

  ```text
  ENVSXP, header flag SLOTS (0x40): 64 + 32n bytes
  +16 overflow   +24 ENCLOS   +32 shape   +40 version box   +48 caller
  +56 meta: capacity u32 | next stamp u32 (an overflow block: slots used)
  +64 slot[n]: key (symbol | kind, bits 0-3) · val · aux · meta (stamp u32 | flags)
  ```

  - The plain-value read is one test on the key: kind 0 is VALUE.
  - Unbound and cell are kinds of their own, off that path.
  - The stamp is in the slot's meta word, outside the key; so are the
    flags (LOCKED, ASSIGNMENT_PENDING, MISSING, PRSEEN, ORIGIN). Fable's
    map had them in the key's top 16 bits; here no pointer bit is assumed
    free beyond the 4 low bits of a 16-byte aligned symbol (M1's heap
    aligns symbols to 16, not 64).
  - Kinds: VALUE, REAL/INT/LGL (unboxed), UNBOUND, DELAYED, FORCED,
    ACTIVE, CELL. Missing is a VALUE holding R_MissingArg with MISSING 1;
    missing-default is DELAYED of ORIGIN self with MISSING 2; locked is a
    flag on any kind.
  - 32-byte slots, against Fable's 16: `aux` holds a delayed or forced
    argument's expression, so a slot needs no call-site code object (V1's,
    not yet there) to know its source. A three-slot frame is 160 bytes;
    the objects GNU R makes for the same call (an environment, three cells,
    three promises) are 336 at robigo's 48-byte node size, and Fable's map
    is 96. X1 (D2) measures the variants
    and swaps `slots.rs`.
  - The collector traces `val` for VALUE, DELAYED, FORCED, ACTIVE and CELL,
    and `aux` for DELAYED and FORCED; keys never. That is the whole rule
    (`slots::val_traced`, `aux_traced`).
- Decisions:
  - **Overflow is the default for names outside the shape.** It is a block
    of the same map, made on the first such name and replaced by a larger
    one when full. Its slots move, with their references uncounted twice.
    No slot stays in a block that is replaced: a binding location is never
    held across a definition, and `R_findVarLocInFrame` turns a slot into a
    cell first. Shape transitions (D2s) are not here.
  - **Listing order is by definition stamps.** A stamp is set when a slot
    becomes bound, never on reassignment, and listing goes stamp
    descending. `new_closure_frame` stamps the formals in reverse order.
    When the per-frame counter would wrap, the bindings are renumbered.
  - **Arguments.** `SlotInit` states follow `matchArgs`/`applyClosure`:
    - supplied → VALUE, MISSING 0 (1 for R_MissingArg);
    - missing → R_MissingArg, MISSING 1;
    - default → DELAYED in the frame, MISSING 2;
    - lazy → DELAYED in the caller, optionally with the value the caller
      computed early (§6.2);
    - forced → FORCED.

    The caller's environment is one frame word, released when no delayed
    slot needs it any more. GNU R drops a forced promise's PRENV, so a
    callee's frame does not keep its caller's alive (finalizer timing is
    observable).
  - **Materialization and expansion happen once and are stored back.**
    Reading an argument slot as an object (`findVar`,
    `findVarInFrame3`, `FRAME()`, `R_findVarLocInFrame`, `findFun3`'s
    forcing) turns it into a VALUE holding the PROMSXP GNU R's binding
    would hold:
    - delayed → unforced, PRENV the caller or the frame, PRSEEN copied, an
      early value in the promise's 4th word for C1's force;
    - forced → forced, PRENV R_NilValue.

    MISSING stays on the slot. Forcing is the registered evaluator's (C1
    ports `forcePromise`). An unboxed scalar is boxed and stored back
    (`R_expand_binding_value`). A locked slot's new object is marked not
    mutable, as `LOCK_BINDING` marks a cell's CAR. `findFun3` reads a
    forced argument's value without making a promise.
  - **Cell-backed bindings.** `R_findVarLocInFrame` moves a slot's value and
    bits into a new binding cell and makes the slot CELL for good. Every
    later read and write goes through the cell. `rm` detaches the cell as
    `RemoveFromList` does (R_UnboundValue, locked), and a reinsertion makes
    a new binding. `R_SetVarLocValue` cannot name the frame, so a
    function-like change through it moves the search epoch.
  - **`FRAME()` is a snapshot.** Each cell carries a binding's value
    (materialized first), symbol and bits; writes to it do not reach the
    frame. `SET_FRAME` on a slot frame is an error. A hashed frame's
    FRAME is R_NilValue, as GNU R's.
  - **`<<-`'s memo** (`SuperAssignCache`) records the shapes of up to four
    frames from ENCLOS(rho) and the slot index where the binding sits.
    The frames before it must have shapes that lack the symbol, so a frame
    of the same shape cannot bind it there. A hit is one shape compare per
    hop, an overflow check, then GNU R's `SET_BINDING_VALUE` on the slot.
    The memo holds its shapes as objects (`#[derive(Trace)]`): its owner
    traces it.
  - **`Loc` stays one word.** A slot location is the owner's address with
    the index in bits 48-62 and bit 0 set (strict-provenance `map_addr`).
    The global cache never holds one: a binding in an attached slot frame
    is found again by the walk.
  - The heap's `env::frame`/`hashtab` shims assert in debug builds that
    they are not applied to a slot frame. N1a's `kind()` now asserts that
    an environment is hashed, base/empty or a slot frame.
- Left to others: `forcePromise` on slots (C1/V1: PRSEEN's "promise
  already under evaluation" and the restart warning), `R_GetBindingType`
  and `.Internal(getBindingType)` over the kinds (C1), dots (C1),
  `addMissingVarsToNewEnv` (C1, S3 dispatch), ESCAPED and frame recycling
  (D5), the VM's slot-index access (V1).
- Proof (WSL2 workstation, Ryzen 9 5900X, 24 threads, rustc 1.100.0;
  GNU R 4.6.2 from the oracle image, never the host's 4.6.1):
  - `tests/frames.rs`, 12 tests, against `tests/oracle/envir-frames.R`
    (recorded):
    - closure frames list newest first with the formals last (`y z a b`);
      `rm` and reinsertion; an unbound local is invisible to `exists` and
      skipped by lookups;
    - overflow bindings interleave with shape locals through growth
      (`o6 o5 o4 o3 o2 o1 n r x q a`; `G.re`, `K.ls`, `E.ls`); `mget`;
    - every argument kind; materialization once, as GNU R's promises,
      releasing the caller;
    - `findFun3` forcing a delayed argument and reading a forced one;
    - unboxed scalars expanding once;
    - a cell-backed binding surviving later writes both ways, and `rm`
      detaching its cell;
    - the FRAME snapshot and the SET_FRAME error;
    - the `<<-` memo's hits and misses;
    - locks, active bindings, versions and NO_SPECIAL_SYMBOLS on slot
      frames;
    - stamp renumbering;
    - collections.
  - `tests/env.rs` (N1a's 15) passes unchanged on slot frames, including
    the oracle's unhashed listing cases.
  - Heap: `each_type_traces_exactly_its_pointer_words` has rows for a slot
    frame with one slot of every kind and for an empty one.
  - Sensitivity, under both normal runs and `ROBIGO_GCTORTURE=1`: untracing
    `aux` of DELAYED, `aux` of FORCED, `val` of CELL and ACTIVE, the caller
    word, or the overflow word each makes
    `slot_frames_keep_their_objects_alive` fail (the last one crashes).
  - `cargo test --release -p robigo-heap -p robigo-rtcore` passes, and so
    does the same under `ROBIGO_GCTORTURE=1`. Miri (`cargo +nightly miri
    test -p robigo-rtcore --test frames --test env`) passes. Clippy
    `-D warnings` on the rt crates is clean.
  - R-level cases `tests/rt/env/{closure-frames,unhashed-env,hashed-env,binding-errors}.R`,
    recorded on 4.6.2 with `scripts/oracle-run.sh 4.6.2
    scripts/regen-expected.sh` (`tests/rt/R-4.6.2/env/`). They have no
    `passing-rt` marker: robigo-rt cannot evaluate R yet.
- Cost, a baseline for V1/X1: instructions per operation, callgrind
  N = 10^5 against 2N, release, at `8153a67f`. Each loop's root binds and
  owner checks are included, and they are not the same work as N1a's
  table: N1a's `frame` read had no `Binding` owner check.

  | op | instructions |
  |---|---|
  | `frame_binding` + read of a shape slot (four-slot closure frame) | 167 |
  | `define_var` replacing that value | 384 |
  | `define_scalar` there (unboxed store) | 157 |
  | `new_closure_frame`, three slots, two arguments | 488 |
  | `find_fun3` through three closure frames to a base function | 397 |
  | `set_var_super`, two frames up, memo hit | 330 |
  | N1a's `find_fun3` through three shapeless frames (overflow) | 365 (326 on pairlists) |

  About 40 of a lookup's instructions are the generic walk
  (classification, then the key scan). The VM addresses slots by index
  and skips it (V1).
