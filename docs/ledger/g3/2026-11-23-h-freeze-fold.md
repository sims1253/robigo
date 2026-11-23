## 2026-11-23: the boot heap's freeze folded into its collection, and on by default (step 2)

Step 1 (#984, the frozen-reachability pass) left two clauses of #959's amended (e) failing with the freeze on: the first collection at 3.8–4.2× main's, and the per-collection average carrying one pass in young.R under rdb. The foreman approved step 2's phase boundary (points 1–7 of H's message of 2026-11-23) with these conditions: the ordinary mark loop untouched, proved by its normalised instruction count, young.R's per-collection count with the freeze off and rt-bars peak RSS; phase 1's work stack released; phase 1 records the frozen objects that meet a not-yet-frozen child; a case for point 3; point 6's CI steps stated; (a) a Globals epoch with a test each way; (b) base's lazy-load environment as an explicit freeze root, with its size (built as a counted exemption, which roots nothing: below); a direct visitor in the pass, measured against about 225 instructions per frozen object; and the default flips when all of amended (e) holds on the three boots and the detach case matches.

### As built

1. **Phase 1** (`Core::freeze_from`, `FreezeMark`): the freezing collection's first step, before the ordinary marking, which is unchanged. Each object is decided when an edge first shows it, with one read of its block:
   - frozen at once: `IMMORTAL | BARRIER | FROZEN` in one flag store, the block's count, the shared reference count, the frozen list;
   - or stopped: a kind never frozen (weak references, their keys, external data, `SEEN_BY_C`, `NO_FREEZE` raw vectors), an object after the boundary, or `exclude`. A stopped object's visited bit is its mark bit (a large one's base in a set), cleared before phase 2.
   - A frozen object that shows a stopped child is logged at that moment (the foreman's choice: no second trace of the frozen set).
   - The frozen list is the walk's queue: no work stack. The stopped list and set are dropped when phase 1 ends.
2. **Phase 2** is the ordinary marking. It stops at frozen objects as at any immortal one, and marks from the log, so a stopped child phase 1 met is marked from its logged parent whatever the other roots do.
3. **The pass's walk** (`PassWalk`): the same shape, a direct visitor, the reached list its queue.
4. **(a)** `Core::frozen_epoch` counts the decisions of the frozen set (the freeze, each pass). `Globals` keeps a bit per slot filled at the current epoch, and releasing such a slot takes no note: no decision saw it, so none depended on it. Before the first decision nothing is tracked (nothing is frozen, so no release can need a note): with the freeze off, a fill costs one epoch load.
5. **(b)** `makeLazy(vars, vals, expr, eenv, aenv)` records `eenv` when it binds at least one name into an unlocked base (its environment or namespace) while base's own file (`library/base/R/base`) is read (`Cx::while_reading_base_file`, closed however the file's run ends), before the boot boundary. Under the rdb boot that is `..lazyLoad`'s frame (`this`, baseloader.R:78), which every promise of base's database is to be evaluated in (`Cx::note_lazy_load_frame`). The boundary hands the recorded frames to the heap with the boundary itself, in one call that a repeat refuses whole (`Core::freeze_boundary(exclude, pins)`).
   - **A pin roots nothing** (Sol step 2 r4 S15). It is a frame with a count: the frozen promises whose PRENV it is that the freeze roots show directly (base's bindings' values; a promise reached through the frame itself would certify only a cycle, Sol step 2 r5 S18), counted by phase 1 and rebuilt by every pass. A frame that stayed ordinary is no pin (r5 S19). A counted promise's PRENV cleared over it (forcing one of base's promises: `gc::store_slow`, the promise's own PRENV slot only, and not a write of the frame back, r5 S20) takes no note and drops the count (a lower bound between walks: an uncounted promise's clear drops it too, which only brings a pass early, Sol step 2 r6 S21); removing a counted promise from the roots (a base binding rebound) is noted as any root's change is. The last counted promise's clear unpins the frame with a dirty note, so the next pass frees the frame and what only it holds if nothing else reaches it: as GNU R's collector does once every promise using the frame is forced. Every other overwrite of the frame notes as usual. A frame no frozen promise uses (a weak key, say) is no pin, and a weak reference keyed on a pinned frame unpins it with a note (`Core::request_unfreeze`).
   - **Only base's own loader:** the system profile, which also runs before base is locked, records nothing (Sol step 2 r2 S8, r3 S12). The site and user profiles run after the locks.
   - **What the frame holds:** reachable only through it (base's promises' PRENV edges set aside), measured at the freeze: 597 objects, 55,776 bytes under rdb with the JIT on (677 objects, 55,968 bytes with it off). Raw vectors 24,736 bytes (the database's keys), pairlists 15,024, lists 6,288, calls 6,240. None under the interim boot, which records no frame. As in GNU R, they live while any of base's 1,200-odd promises is unforced: in practice, the session.
6. **Pinning without a frozen child is not dirty.** A symbol installed from a frozen CHARSXP pins it (Sol r1 S1), and the pin noted dirty (Sol r2 S8). A CHARSXP has no children, so the note is now taken only when the pinned object has a frozen child. This was the remaining pass in young.R under rdb with the JIT on.
7. **Globals reserved at the start's size.** The start's peak is 3,220 slots (interim), 2,986 (rdb, JIT on) and 2,904 (rdb, JIT off), so 4,096 are reserved and the table never reallocs during a start (X's 29k).
8. **The default flips, in the PR's last commit and only there** (the foreman's condition): `ROBIGO_GC_FREEZE` unset or `1` is on, `0` is off, with rt-ci's step renames and test-rt-ci's assertions. Reverting that one commit keeps the fold with the freeze off by default.
9. **A panic inside either freeze path poisons the heap** (Sol step 2 r1 S1): `poisoned` is set before the lazy freeze starts in its collection, and around the eager `Core::freeze`. A poisoned heap's next collection, and its next eager freeze, panic with "robigo: this heap was left halfway by a panic during a collection's marking or moving: it can only be dropped" (the eager freeze's message names the panic only). robigo-rt catches no panic: one out of a collection unwinds to `main` and ends the process. quit()'s guard is the one `catch_unwind`, and it aborts. So only an embedder that catches the panic (the heap's own tests) ever holds a poisoned heap. No R-level error path reaches it: R errors are `RResult` values, not unwinds, and the collector runs no R code (finalizers run after it).

### Point 6: the CI steps after the flip

- Every conformance and crate-test step that names no `ROBIGO_GC_FREEZE` now runs with the freeze on, its check off: the plain conformance under each boot, the crosscheck, the observation modes, frame quarantine, retry torture, poison-dead, the builtin differential.
- The freeze's checked steps name it on: gctorture with the check at every collection (the crates' tests), E3 at every 16th, and the nightly's every-collection check.
- Two steps test the heap without it: `conformance with the boot heap's freeze off (ROBIGO_GC_FREEZE=0)`, in the gctorture shard, and its rdb twin in modes.
- The moving collector's steps turn it off explicitly: `ROBIGO_GC_MOVE=1` with gctorture (the crates' tests), `ROBIGO_GC_MOVE=young:N` (E3), and the nightly's full move. `scripts/test-rt-ci.py` checks that every step with `ROBIGO_GC_MOVE` sets `ROBIGO_GC_FREEZE=0`.

### Measured (callgrind, the PR's head against main 0f2a40371 with the freeze off)

**Amended (e), freeze on:**

| clause | interim | rdb, JIT on | rdb, JIT off |
|---|---|---|---|
| startup instructions, mean over 15 R_HOME lengths (within 1%) | −1.14% | −0.03% | +0.08% |
| startup anonymous memory, mean over 15 lengths (within 1%) | −0.49% | +0.28% | +0.17% |
| per collection after the first, young0.R (at most 2.70M) | 2.09M | 2.26M | 2.07M |
| the first collection (at most 2× main's) | 1.38M / 0.79M: 1.76× | 8.68M / 5.07M: 1.71× | 4.50M / 2.79M: 1.62× |

young0.R and young.R run no frozen-reachability pass under any boot (step 1: one under each rdb boot).

The two workloads (Sol step 2 r3 S14): **young0.R** is young.R with an `invisible(gc())` first, so its first of 6 collections is the freezing one. Its per-collection figure is (the collector's total − the first collection) / 5, the first collection measured alone by firstgc.R (`invisible(gc())`). **young.R**, 5 collections, its first the freezing one, gives the collector totals below. Each figure is callgrind's inclusive `Core::collect_needing` over the whole run.

**Beside amended (e)** (Sol step 2 r2 S10):
- **Startup peak RSS (`VmHWM`, at most +5%)**, the mean over the 15 lengths: freeze on −0.83%, +0.17% and +0.12%; off −0.83%, +0.23% and +0.14%.
- **young.R's collector total beside main's** (5 collections; 12.38M, 29.25M, 19.07M): freeze on 11.52M (−6.9%), 19.61M (−33.0%), 14.47M (−24.1%); off 12.37M (−0.07%), 29.25M (+0.00%), 19.05M (−0.10%).
- **rt-bars** (freeze on, against main 0f2a40371; `perf.py rt-bars` in a gate slot): 0 failures, all 28 rows within −0.07% to +0.17%. The three programs' peak RSS: binarytrees 26,504 KiB against 26,996; listgrow 46,752 against 46,768; churn 80,440 against 80,528. Its one-path starts: interim −1.72%, rdb with the JIT on +0.04%, off −0.47% (an earlier build read +0.52% there: the path effect below).

**The freeze off:**
- startup instructions, mean over the 15 lengths: −1.13%, −0.07% and +0.10%; anonymous memory −0.47%, +0.42% and +0.25%;
- young0.R's collector (6 collections): −0.28%, −0.05% and −0.09%. Per collection after the first: 2.458M against 2.464M, 5.584M against 5.588M, and 3.644M against 3.648M. (An earlier build read the interim +0.22%: the difference was in `prune_chars`'s `calloc` and the sweep's `memset`s, the same calls in the same number, which cost what the heap's layout gives them.)
- **the mark loop:** `Core::trace` is the same 0xedd bytes as main's, its normalised disassembly the same but for one PC-relative constant (a panic location). `mark_rest` is smaller than main's: the freeze check's mark-time walk (`check_frozen_marked`, run only under `ROBIGO_GC_FREEZE_CHECK`) is out of line.

**Phase 1's cost:** about 4.9M for 42,468 frozen objects under rdb with the JIT on, about 116 instructions per frozen object (step 1's separate walk: 16.6M; measured on an earlier build of this branch, before the pins stopped rooting).

**A pass:** 4.43M under rdb with the JIT on, 104 instructions per frozen object (step 1: 7.96M, 187 per object; the same earlier build).

**Layout:** a single start's count steps with the heap's layout (#984's paragraph): over the 15 R_HOME lengths main's own starts range over 14.75M–14.88M, 92.46M–93.17M and 49.94M–50.22M, and its rdb anonymous peak over 6,812–6,960 KiB. The means above are the comparison. A single path can read +1.3% anonymous memory for either binary (rt-startup.py's default path, rdb with the JIT on, read +88 KiB for one intermediate build of this branch and +4 KiB for another).

**Detach:** `freeze-garbage-detach.R` matches GNU R with the freeze on (step 1).

**Point 3's case:** `tests/rt/memory/freeze-phase1-parent.R` (recorded on GNU R 4.6.2): an object made after the boot, stored in Autoloads before the first collection, whose only path goes through that frozen frame.
