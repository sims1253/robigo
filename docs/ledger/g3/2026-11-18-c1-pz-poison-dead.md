## 2026-11-18: C1-PZ: dead-slot poisoning (`ROBIGO_POISON_DEAD=1`)

D9 clears a register at its last use, so that no dead slot keeps an object
alive across a later safepoint (`ir.rs`, "Dead registers"). A clear that
comes too early leaves an empty slot. A later read then takes the empty
slot for a missing value or an unbound one, and nothing fails where the
bug is. C1-PZ (m2m3-plan.md, wave 1) makes such reads fail where they
happen.

### What landed

- **robigo-rtcore's `poison-dead` feature**, forwarded by robigo-rt's
  feature of the same name. It is compiled in only with the feature, as
  `retry-torture` is. With `ROBIGO_POISON_DEAD=1`:
  - `Slot::dead(reg, pc)` is a poison. Its tag word is `MARK + 1`, with
    the register and the pc packed in, and its payload is the
    `&'static Location` of the source line that killed the slot. Without
    the feature or the variable it is `Slot::EMPTY`.
  - `ValueStack::restore` (a scope's or an activation's frame popped)
    poisons the slots it frees, naming each slot's index.
  - `Slot::check_live` panics on a poison:
    `robigo: ROBIGO_POISON_DEAD: a read of a dead slot: register R,
    cleared by instruction N at vm.rs:L:C` (or "value-stack slot I, freed
    by a scope's restore"). `ValueStack::read` (every `Root`, `Args` and
    `Out` read), `Act::get` (every VM register read), the inline
    evaluator's register reads and `Slot::tag` (a slot of an unknown tag)
    call it.
  - The collector and the link walk read slots raw and match exact tags
    (`as_object`, `as_mark`), so a poison is neither traced nor taken for a
    mark.
- **robigo-rt.** D9's clears are `Act::kill(r, pc)`:
  - in `vm::run`, F_CLR_B/F_CLR_C (`clear_operands` and the arms that clear
    by hand), `Clear`, `ClearRange`, the Ret arms' value register, and the
    scratch register after `EvalConst`, all with the instruction's pc;
  - outside `run`, the scratch registers after a raw-layer call, argument
    windows after a call, a loop region's temporaries, `Move`'s source, and
    `assign`/`bcassign`'s right-hand side, all with no pc
    (`u32::MAX`) but with the source line;
  - the inline evaluator's `Clear` and `ClearRange`.

  `Act::clear` stays for slots that are written again, not dead: an
  `Out`'s slot, and `apply_special`'s destination.
- **The plant.** With the feature, `ROBIGO_POISON_PLANT=1` makes the
  compiler empty each closure's value register just before its `Ret`.
  `crates/robigo-rt/tests/poison.rs` runs code clean under the poison, then
  with the plant in both profiles. The VM and the inline evaluator each
  panic with "register R, cleared by instruction N".
- **rt-ci.** The retry-torture step's build is now made with both
  features (`CHECK_FEATURES`), and two steps follow it under
  `ROBIGO_POISON_DEAD=1`: robigo-rt's tests (the plant among them) and the
  conformance. Every step before them still tests the binary users get;
  `test-rt-ci.py` checks that the featured steps are the last three.

### Results

- Conformance under `ROBIGO_POISON_DEAD=1`: 578/584. The 6 failures are
  main's unratcheted cases (5 `bytecode/*`, `library-dynam-tools`). No
  poison read: D9's clears and the restores are all at or after the last
  read on today's slice.
- robigo-rt's tests under the poison pass, and so does the plant test.
- Miri on robigo-rtcore (default features) passes.
- Callgrind quick check against main `bcd7fe548`'s binary, in
  instructions per iteration: the production build is unchanged (the
  feature is off).

  | row | main | C1-PZ |
  |---|---:|---:|
  | plain | 1898.4 | 1898.2 |
  | named | 1845.5 | 1845.8 |
  | defaults | 1983.9 | 1984.8 |
  | env-fn | 1938.6 | 1938.8 |
  | superassign | 4905.7 | 4904.7 |
  | guard-subassign-dbl | 5110.0 | 5110.6 |
  | d0-names | 5435.7 | 5434.9 |
  | guard-subset-dbl | 1739.8 | 1739.1 |
