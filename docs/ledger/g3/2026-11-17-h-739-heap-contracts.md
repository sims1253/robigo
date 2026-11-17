## 2026-11-17: robigo-heap's contracts: header setters, moving addresses, collection panics (#739)

- Why (#739, from an unsafe-code review of `9eb655df`): robigo-heap's
  contracts had gaps that unsafe callers (rtcore, rtapi) could meet and
  still corrupt the heap.
  - A header setter could hide a bit the collector reads:
    `set_extra(0)` on a tagged promise makes PRVALUE's raw bits an edge;
    `set_flag(BARRIER, false)` on an immortal object skips the remembered
    set.
  - The moving debug collector broke the documented stability of payload
    pointers and CHARSXP byte slices.
  - `set_gc_move` was safe, although turning it on late moves what C may
    hold.
  - A panic while marking or moving left forwarded objects and
    `collecting` set.
  - Several contracts left a condition unstated.
- What:
  - **Header setters.**
    - `set_flags` and `set_flag` must not change IMMORTAL or BARRIER (the
      heap's): its contract says why and debug builds check it. The heap
      itself writes them through `set_flags_raw` (`make_immortal`, the
      barrier's slow path). SLOTS, ALTREP and EXTDATA stay the caller's to
      change, as the contract says: FRAME_QUARANTINE clears SLOTS on
      purpose.
    - `set_extra` must keep the sticky SEEN_BY_C and a promise's tag,
      debug-checked.
    - `set_promise_tag` and `set_promise_bits` are crate-private. The
      public pair is `set_promise_unboxed(tag, bits)`, beside
      `set_promise_value`, so the collector never sees a tag that disagrees
      with PRVALUE.
  - **The moving collector's address rule.** `Sexp`'s doc defines it: an
    address is stable only between collections, and a derived address (a
    payload pointer, a byte slice) lasts until the next collection unless
    the object is immortal or handed to C. "Live" now also says the
    `Core` still exists. `std_data`'s and `char_bytes`' postconditions and
    `install_char`'s SAFETY reason follow the rule.
  - **`set_gc_move`** is `unsafe` in the heap and in rtcore. Stopping has
    no condition; starting needs that C holds no address handed to it
    while moving was off.
  - **Collection panics.** An abort-on-unwind guard covers marking,
    evacuation and forwarding (`collect_needing`). The sweep stays
    unwindable: a heap test panics mid-sweep, and dropping the heap still
    runs every release once. `collect`'s contract adds that `roots` shows
    the same edges each time it is traced in one collection (a moving
    collection traces twice).
  - **The other gaps:**
    - `scan_top_slot` and `robigo_heap_try_call65` need a core that neither
      moves nor is dropped;
    - `set_poll_word`'s word is accessed atomically only;
    - `alloc_extdata`'s data is aligned for `t`;
    - the CHARSXP table's power-of-two length is stated on the field and
      debug-checked in `find` and `rebuild`.
  - **#765's deferred lows** (OCR round 3):
    - a heap test pins `release`'s pairing with one `preserve` of its own
      object;
    - the `Sexp` doc states the Sexp check's coverage and its gaps (a
      `Sexp` reached through another public type or an alias);
    - call.rs names the contract that covers `fun` (register `rf`'s
      object), not only `call`'s;
    - `arith_nums`' caller list drops `fused`;
    - `vm::num`'s contract states liveness while read, with a register of
      a running activation as one way to meet it.
