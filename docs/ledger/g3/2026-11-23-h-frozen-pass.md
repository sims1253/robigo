## 2026-11-21: the frozen-reachability pass's bar (closes the boot freeze's INTERIM(H) "frozen garbage stays a root")

Written before any build (2026-11-21), accepted by the foreman with its amendments (2026-11-22: the deferred search-path test, the compiler frozen under the rdb boot), and signed in the pull request that adds this entry. It amends `2026-11-21-h-boot-freeze-bar.md` (#948) and closes the INTERIM(H) of `2026-11-21-h-boot-freeze-check.md` (#959).

### The problem

After #959, a frozen object is frozen for the whole session. A frozen object whose last reference is cut stays in the heap (bounded by the frozen heap, 1.7–1.9 MB under the rdb boot). If it is logged, it is also a root for good: it keeps its ordinary children as of its death, and they are never finalized. `tests/rt/memory/freeze-garbage-detach.R` differs from GNU R 4.6.2 for this reason. Autoloads is attached by the system profile and so frozen. A binding added to it later is ordinary, so Autoloads' hash table is logged. After `detach`, the dead table still holds the binding's environment.

### (a) The pass

- **The graph.** The pass walks the freeze roots of #948 (a) and the edges between frozen objects. The roots are the base environment and namespace, `SymCells`' values and `.Internal`s, the namespace registry, `Globals`, the C API's constants, and the search path from the global environment's ENCLOS. It is the freeze's own walk (`Roots::trace_freeze_roots` and the frozen fields), so a pass run at the freeze point thaws nothing (premise 3).
- **When.** At the start of the next collection of any kind after a dirty event (the foreman's decision C), before marking. The heap's `Remembered` holds a dirty byte, because the barrier's slow path already loads it.
- **What it does, in one walk:**
  1. Objects queued for unfreezing (`reg.finalizer`, `R_MakeWeakRef`, `R_RegisterCFinalizer*`, and `DATAPTR` from (b)) leave the graph first: the walk treats them as ordinary.
  2. Walk from the freeze roots, following edges whose target is frozen.
  3. Every frozen object the walk does not reach becomes ordinary (`make_mortal`). Its count stays at the frozen maximum (shared, sticky, as #959's thaw does), and it leaves `Core::frozen` and the log.
  4. The frozen part of the log is rebuilt as every reached frozen object that has an ordinary child, plus every object #959's hardening logged.
  5. The dirty byte is cleared.

  Marking then runs as now. A thawed object that is still reachable through ordinary objects or the stack is marked and traced from then on. One that is not is swept, and its finalizer runs.
- **This replaces `unfreeze_pending` and `thaw_with_referrers`.** One walk serves the unfreeze queue and the frozen garbage, so the heap keeps a single mechanism that changes the frozen set. This is a deeper module than two passes with two invariants.
  - **What changes:** #959 thaws a queued object and its frozen *referrers*. The one walk thaws a queued object and the frozen objects reachable *only through it*. Its frozen referrers stay frozen and are logged, because it is now an ordinary child.
  - **Why the referrer thaw is no longer needed:** it existed so that a log entry could not decide a weak key's death (Sol S3, #959). With the pass, a frozen referrer that becomes garbage is thawed at the next pass, because cutting its last edge is a dirty event. So a log entry keeps a key alive only while its frozen referrer is really reachable, as in GNU R.
  - **The freeze's own thaw at its end** (the weak keys' and external data's frozen referrers) goes too, replaced by logging. Premise 3 measured what it costs today: under the rdb boot it thaws 20 referrers and strands 134 frozen objects (574 words) that are reachable only through them.
  - **What one walk thaws for a queued container** is measured in (h).
- **Why the thawed set is right.** A frozen object that is reached only through a thawed or ordinary object is thawed too. It is still traced through that object, so nothing reachable is lost. A kept frozen object never points to a thawed one: the walk follows every frozen edge, so a kept object's frozen children are reached and kept. Step 4 logs every kept object with an ordinary child, so #948's invariant holds after the pass.

### (b) Dirty events, and the proof that the table is complete

**Claim.** The frozen-reachable set shrinks only when an edge into a frozen object leaves the graph:
- a **frozen-to-frozen edge** is overwritten: a store into a frozen object whose old value is frozen;
- a **root-to-frozen edge** is overwritten: a freeze root rebound or released while its old value is frozen;
- a **frozen node becomes ordinary:** the unfreeze queue is not empty.

Adding an edge never shrinks the set. Overwriting an ordinary value, or an immortal value that is not frozen (a symbol, `R_NilValue`, a PRINTNAME), removes no edge of the graph. So these three events, and only these, set dirty.

**This is narrower than the foreman's B** ("a store into a frozen object"), and still sound by the claim. The difference matters for the in-place `SetOption` port in (c): every `options(x = )` call stores into a frozen cell, but only a call that overwrites a frozen value sets dirty, and a pass then thaws that value.

**Rejected:** "the overwritten value is frozen, in any container." It grows with N in 34 of the 120 row and loader runs. For example, `nenv` has 56,001 such stores at N and 112,001 at 2N, from frame slots overwritten each iteration while they hold base functions and environments (H.md §27). An ordinary container's edge is not in the graph, so those stores need nothing.

| path | the edge it can remove | how dirty is set |
|---|---|---|
| heap setters into a frozen object: `set_car`, `set_cdr`, `set_tag`, `set_elt`, `set_attrib`, `set_aux`, `set_promise_value`, `set_enclos`, `set_frame`, `set_hashtab`; slot frames' `store_val`, `store_aux`, `store_val_uncounted`; `layout::set` | the field's old value | all go through `Sexp::store` or `store_uncounted`. The inline fast path tests `flags & (BARRIER \| FROZEN)`: one test of the byte, as now. The slow path sets dirty if the old value's header has `FROZEN`, and logs the object if `BARRIER` is set (then clears `BARRIER`) |
| the C API's setters: `SETCAR`, `SETCDR`, `SET_TAG`, `SET_ATTRIB`, `SET_VECTOR_ELT`, `SET_STRING_ELT`, `SET_PRVALUE`, `SET_FORMALS`, `SET_BODY`, `SET_CLOENV`, `SET_ENCLOS`, `R_SetExternalPtrProtected`, `R_SetExternalPtrTag` | the same | through the row above. #948's premise 2 grep is rerun on the PR's base: no raw setter outside robigo-heap |
| writes of no pointer: `set_val_bits`, `clear_raw_val`, `set_key`, `set_promise_unboxed` / `set_promise_bits`, `set_extptr_addr` | none: each writes a word that holds no traced pointer | `store_scalar` and `unbind` (`env/frame/mod.rs`) store NULL through `store_val` before they change a slot's kind. An unforced promise's PRVALUE is `R_UnboundValue`, which is immortal and not frozen. The extptr address is untraced, and a key is a symbol. **Test:** a grep of every caller; each is preceded by a barriered store or runs on a raw-kind word |
| a base binding: a `SymCells` value (`env/mod.rs`'s base-binding write, `cell_mut` with `mem::replace`) or `set_internal` | a root edge | the write tests the old value's `FROZEN`. `set_internal` runs at boot only, so a debug assertion checks it runs before the freeze. Measured: 0–1 per run, constant in N |
| `Globals::remove` | a root edge | tests the released slot's `FROZEN`. Measured: 1–8 per run, constant |
| `set_enclos` on an ordinary environment (the global environment, a user-attached one) | the search-path walk's edge | **a deferred test, not a per-write one.** Any `set_enclos` sets a "path written" byte. At the next collection's start, if the byte is set, the heap walks the search path as the freeze roots do and compares its frozen members with the snapshot taken at the last check. Dirty is set only if a member is gone. Then the snapshot is replaced and the byte cleared. The walk is about 10 environments, once per collection that follows a write.<br>**Why deferred:** `attach` writes as GNU R's `do_attach` does: the global environment's ENCLOS first (to the new environment, whose own parent is not yet set), then the new environment's parent. Between the two writes Autoloads is off the chain, so any test at the write itself sees a loss. The first draft's per-write chain test was measured to dirty twice per attach. Measured with the deferred test: an attach/detach loop of a user list, collecting every 2,000 allocations, has 2 dirty collections (interim loader) or 3 (rdb) at both N = 1,000 and 2,000, against 30 and 58 collections. The one path loss is the final `detach("Autoloads")`. It also removes, without a separate condition, the 538 false events in tests/rt under the rdb boot, from environments off the path.<br>**Why it is sound** is stated below the table |
| **a forced promise** (`set_promise_value`, then PRENV cleared with `set_tag`/`set_cdr`) | PRVALUE: none (its old value is `R_UnboundValue`, immortal and not frozen). PRENV: the frozen environment it held | the first row. **Under the rdb boot, forcing each of base's lazy-load promises sets dirty once** (PRENV over a frozen environment, store kind `PROMSXP+32 old ENVSXP`). The interim loader has no lazy-load promises: 0. Measured below the table |
| **a frozen environment's hash table replaced on growth** (`hashtab::resize` → `set_hashtab`) | the old table | the first row. The old table is garbage and the pass thaws it. Measured 0 times over the conformance and the corpus shard. **A new chain head in a bucket** (`set_elt` on the table, store kind `VECSXP+off old LISTSXP`) overwrites the old head, and the new cell's CDR points to it, so no edge really leaves. It sets dirty all the same: 3 per tests/rt slice and 25 in the corpus shard, all under the rdb boot. Accepted as a rare false positive; a one-hop test (the new value's CDR is the old value) would remove it |
| **attributes replaced** (`set_attrib`, `SET_ATTRIB`) on a frozen object | the old attribute pairlist | the first row. An R-level `attr<-` on a frozen vector or closure copies (a frozen object reads as shared), so only an environment, which is modified in place, or C reaches a frozen object's attributes. Over a nil or ordinary old value: no dirty. Measured 0 |
| **`SET_BODY`, `SET_CLOENV`, `SET_FORMALS`** on a frozen closure | the old body, environment or formals | the first row. R-level `body<-`, `environment<-` and `formals<-` copy a shared closure, so only C reaches these. Measured 0 |
| **the JIT's code cache replaced** (`code::cache_on`, `set_aux` on a call's or closure's 4th word, C1's #943) | the old code object, when it was frozen | the first row. A first cache overwrites nil: no dirty. Replacing a code object compiled during the boot: dirty, once per such cache. Measured (stores of kind `CLOSXP+40`/`LANGSXP+40 old VECSXP`): 51 in tests/rt and 54 in tests/r under the rdb boot, 21 and 27 under the interim loader |
| a frozen pairlist's CDR rewritten (a cell spliced out) | the spliced-out cell | the first row. Measured 44 per conformance slice under the rdb boot |
| **C1's translation-cache stub replaced** (a 16-byte RAWSXP put in a BCODESXP's aux at unserialize, replaced by the code object at the first call or force) | the stub, when it is a boot object (unserialized before the boundary) | the first row: a store into a frozen object over a frozen old value, so dirty once per replaced boot stub, 429 in one JIT-on start, batched into the next collection's pass. That pass thaws the replaced stubs. **Recommended instead:** C1 marks a stub (a RAWSXP gp bit) and the freeze leaves marked objects out, as it leaves out weak keys. A stub then stays ordinary, its BCODESXP is logged at the freeze (it has an ordinary child), and the replacement sets no dirty, because the old value is not frozen. The stub is then collected as usual. The replacement must go through the barrier (`set_aux`, `Sexp::store`) in either case: a raw write would leave the new code object reachable only through an unlogged frozen object, and the check mode would report it |
| `set_enclos` on a **frozen** environment off the search path (a frozen closure's environment re-parented: `parent.env(e) <- …` on a boot environment) | a frozen edge: the old parent | **the first row, like any other store into a frozen object over a frozen old value.** The deferred search-path test does not cover it and is not needed for it. **A case pins it:** `parent.env<-` on a boot environment off the path, with the old parent then unreachable. A finalizer is registered on something only the old parent held, then `gc()`. The finalizer runs, as in GNU R 4.6.2 |
| the C API's constants (`CGlobals`) | a root edge | written at boot only. **Test:** a grep shows no writer after the freeze, or the row gets the base-binding test |
| the namespace registry (a frozen hashed environment) | a frozen edge | the first row |
| `reg.finalizer`, `R_MakeWeakRef`, `R_RegisterCFinalizer*`, `new.env`'s finalizers on a frozen object | a node becomes ordinary | the unfreeze queue is not empty, so a pass runs |
| C writing an element through `DATAPTR`, `VECTOR_PTR`, or a cast `STRING_PTR_RO` | a raw word: **sets nothing** | covered by thawing the object instead of keeping it frozen. C's first writable `DATAPTR` of a frozen STRSXP, VECSXP or EXPRSXP still logs it (#959), which covers raw writes until the next collection. **New:** it also queues the object for unfreezing, so the next pass makes it ordinary and `SEEN_BY_C`, as #948 (a) says objects C has seen are. The pass does not follow an ordinary object's edges. So a raw write into it can strand only ordinary objects, and marking handles those. Cost: one pass per exposed vector, then that vector is traced at every collection. This changes #959's hardening from "logged and kept frozen" to "logged, then thawed". **It is rare by count** (the foreman's item 3): see below the table |
| `FreshVec::put` | none | only on a fresh, unpublished vector, which is never frozen |
| the moving collector | none | off with the freeze. `set_gc_move` and `set_gc_move_young` assert that the frozen set is empty |
| raw writes elsewhere (`*(… as *mut Sexp) =`, `ptr::write`, `copy_nonoverlapping` into a body) | – | #948's premise 2 grep, rerun. Each hit is in this table or on a fresh object |

**The deferred search-path test's scope.** It is the rule for the root chain only, whose links run through ordinary environments. A `set_enclos` on the global environment or on an attached package environment sets "path written", and the comparison at the next collection decides. It covers nothing else. A `set_enclos` on a frozen environment, whether on the path or not, is also a store into a frozen object, and follows the first row.

**Why the deferred search-path test is sound.**
- **The enumeration it depends on:** the search path is a freeze root that the pass enumerates by walking the whole ENCLOS chain from the global environment (`Roots::trace_freeze_roots`, `crates/robigo-rtcore/src/isolate.rs`: from the global environment's ENCLOS until base, the empty environment or a repeat). The walk goes through ordinary environments too, and offers every environment it meets as a root. So the search path's root edges to frozen objects are exactly "the frozen environments on that chain".
- **Why that is enough:** the test compares that set at the collection's start, where the pass would run, with the set at the last check. No collection runs between the writes of one `attach` or `detach`. So if no member was lost, no root edge to a frozen object was lost, whatever happened in between. A write to an environment off the chain changes no member at all.
- **When it stops being sound:** it is sound only under this enumeration. If the pass ever enumerated the search path another way (a recorded list of attached environments, or stopping at the first ordinary one), the test would have to walk the same way, or every `set_enclos` over a frozen value would have to set dirty.
- **Condition of this bar:** the deferred test and `trace_freeze_roots` call one walk function. A test pins it: it attaches an ordinary environment between the global environment and a frozen one, detaches the frozen one behind it, and checks that dirty is set and the frozen one is thawed. A change that gives either its own walk fails the bar.

**Promise forcing, and what it costs a session that keeps touching new base functions** (the rdb boot only). Each of base's lazy-load promises sets dirty once, when it is first forced; after that its PRENV is nil. Dirty events between two collections share one pass. So a session pays at most one pass per collection that follows a first use of some base function, and at most as many passes as base has lazy-load promises in all. Measured, with the scratch probe:
- **The conformance:** forcing clears a median of 14 PRENVs per process (at most 1,100). They batch: 48 of 2,347 collections in tests/rt and 79 of 2,447 in tests/r are dirty (2.0% and 3.2%), at most 7 in one process.
- **The perf rows** (collecting every 2,000 allocations, `R_GCTORTURE=2000`, so that the rows collect at all): the dirty collections are identical at N and 2N on all 60 rows under both loaders. Under the rdb boot that is 674 of 1,209 collections at N and 674 of 1,769 at 2N. The jit rows are the worst: 90 of their 92 collections are dirty, all while the compiler and the code they compile are first used, and none in the loop.
- **So the per-iteration share is zero.** A start-up phase that touches many base functions collects at main's pre-freeze cost, never above it.
- **A refinement, not in this PR:** most cleared PRENVs are frozen lazy-load environments that other unforced promises still hold, so the pass thaws nothing for them. Exempting a write over an environment the walk cannot lose (a pinned set tested in the slow path) would remove most of these events. It is worth doing only if a row shows the start-up passes; measured here, none does.

**`DATAPTR` on a frozen vector of objects is rare by count** (the foreman's item 3):
- **GNU R 4.6.2's own C** takes a writable `STRING_PTR` or `DATAPTR` of a vector of objects only on a vector it has just made or copied: `platform.c`'s and `sort.c`'s sorts, and `memory.c`'s allocation. robigo-rt ports these in Rust; none reaches `api.c`'s `DATAPTR`.
- **The 20 pinned packages' C and C++** (`tests/packages/sources.tsv`'s tarballs): none. The three hits are tidyr's `DATAPTR` fallback for R < 3.5, cli's `STRING_PTR_RO` alias for old R, and a comment in rlang.
- **Dynamically** (a counter at `robigo_rtapi_remember` for frozen objects): 0 over the conformance under both loaders and over the corpus shard.
- **The package gate runs robigo-cli, not robigo-rt,** so no CRAN package's C runs under robigo-rt yet. When one does, the counter goes into that gate's report.

### (c) Cost

- **The fast path is unchanged:** one test of the flags byte against a constant mask, `BARRIER | FROZEN` instead of `BARRIER`. Stores into ordinary objects run the same instructions, and the rt-bars perf gate checks it.
- **The slow path, per store into a frozen object:**
  - the shared-object test (a block-header load and compare);
  - a load of the old value's header and a `FROZEN` test;
  - the dirty-byte store into `Remembered`, which the logging already loads;
  - the `BARRIER` test, then `store_counted`.

  Estimated from the code at 15–25 instructions over the fast path. **The build measures it:** a robigo-heap loop of N and 2N stores into a frozen cell under callgrind, recorded in the PR's entry. The bar is at most 40 instructions per store.
- **Per iteration, on every perf row: zero.** I counted with a scratch probe on #959's head (H.md §27; table in `~/.cache/robigo-research/h-fz-dirty.md`): 60 rows, under the interim loader and the rdb boot, at N and 2N.
  - Stores into a frozen object are constant in N on every row: 0–111 per run. #948's probe found 3,318 for `jit-safe`, also constant and one-time: JIT caches and promises forced.
  - Base rebindings over a frozen value: 0–1 per run. `Globals` releases of a frozen value: 1–8.
  - The dirty events of (b) are a subset of these, so the passes per run are constant in N on every row. The bar reruns the probe on the PR's head and counts dirty events and passes.
- **Builtins and patterns that store into a frozen container on every call.** Probed on #959's head at N = 1,000 and 2,000, under both loaders:

  | loop body | stores into frozen, per run | dirty per iteration? |
  |---|---|---|
  | `op <- options(digits = 3); options(op)` | 0 (m1), 3 (rdb), constant | no. `set_option` rebuilds the list, so only `.Options`' binding is rebound, over a frozen value once (1, constant) |
  | `assign("zz", i, envir = baseenv())` | 0 | no (a `SymCells` value over an ordinary one) |
  | `registerS3method("print", "zzc", …, envir = baseenv())` | 54 / 16, constant | no. The cell is made in the frozen table once, then its ordinary CAR is overwritten |
  | `e <- new.env(); reg.finalizer(e, …)` | 0–9 | no |
  | `attach(list(a = i), name = "p"); detach("p")` | 9 / 6, constant | **no, with (b)'s deferred search-path test** (measured: 2–3 dirty collections at both N and 2N). A per-write test would dirty every iteration, because each attach writes the global environment's ENCLOS over frozen Autoloads before the new environment's parent is set |

  - **After B's in-place `SetOption` port,** every `options(x = )` call stores into a frozen `.Options` cell and pays the slow path. The restore pattern sets dirty once, not every iteration. The first `options(digits = 3)` overwrites the boot's frozen `7L`, the pass thaws it (only `op` holds it now), and from then on `options(op)` writes an ordinary value back. This is predicted here; B's port PR measures it on this loop.
  - **`.Random.seed`** is excluded by rule: it lives in the global environment.
  - **The JIT's and `compile`'s caches on base closures, and base's lazy-load promises,** are written once each.
- **A program that dirties every collection** pays one pass per collection. That would take a loop that overwrites a frozen value which stays frozen-reachable, such as `parent.env(e) <- baseenv()` on a frozen environment, alternating. We know of no perf row or builtin that does this.
  - **What a pass costs:** a walk of the frozen set's edges. That is what main traced at every collection before the freeze: about 1.25M instructions under the rdb boot (#948: one `gc()` 1.84M, frozen-aware 0.59M). On top of that comes the log rebuild, which is bounded by the frozen objects that have an ordinary child.
  - **So it is no worse than main before the freeze.** The bar measures it with a row reported, not gated: `young.R` with one dirtying store per iteration. Its collector total must be within 10% of main-before-freeze's for the same script.

### (d) The invariant after a pass, and what the check covers

At every collection, after the pass step:
- **I1 (#948's):** every ordinary object that a frozen object points to is reachable from a root. A frozen object with an ordinary child is in the log.
- **I2:** every frozen object is reachable from the freeze roots through frozen objects. This holds exactly at the end of a pass. Between passes it holds unless a dirty event happened, by (b)'s claim.
- **I3:** `FROZEN` is set exactly on the members of `Core::frozen`, and each of them also has `IMMORTAL`.

`ROBIGO_GC_FREEZE_CHECK=N` (#959's period: E3 at 16, the crates' gctorture tests and the nightly at 1) checks all three at each checked collection:
- I1 as now: the direct per-edge check, and the comparison with the independent trace.
- **I2, new:** an independent walk from the freeze roots through frozen objects.
  - When the dirty byte is clear, every frozen object must be reached. A panic names the unreached object and says that no dirty event was recorded since the last pass. **This catches a missing row in (b):** an edge removed without setting dirty leaves an unreached frozen object at a clean collection.
  - When dirty is set, the pass runs first, so the check always runs on a clean state.
- **I3:** the bit agrees with the set.

Failures go to `ROBIGO_GC_FREEZE_CHECK_REPORT`, which rt-ci fails on (#959's S14).

**A heap-level test** cuts edges by each path of (b) that is reachable from robigo-heap: a store, a root rebind, `set_enclos`, the unfreeze queue, and `remember`. For each, it asserts dirty, the thaw, and I1–I3. Miri runs it.

### (e) The header bit

- **`flags::SPARE` (0x20) becomes `FROZEN`.** It is the flags byte's only free bit.
- **The moving collector never sees it.** The mover uses the same bit as `OLD`, but the freeze is off under both move modes, and the mover cannot start after the freeze (the two `set_gc_move*` assertions). The mover's mark path also tests `IMMORTAL` before `OLD` (`mover.rs`).
- **It is free under all four layouts of X3's 2×2.** D8 changes the vector header after the 16-byte object header, and D13 changes node sizes. The object header (type, flags, gp, refcnt, extra) is the same in `h32`/`h24` × `n48`/`nT`. The h24 cells bit is in `extra`, not in the flags byte. The layout decision is still the lead's, and none of its options uses this bit.
- **The C API never reads it.** `OBJECT` reads `flags::OBJECT` only, `LEVELS` reads gp, and no export reads the flags byte whole. Serialization writes sxpinfo from type, gp and `OBJECT`.
- **The doc comment is updated.** It kept the bit "for RGenGC's write-barrier-unprotected objects", but that role is `extra::SEEN_BY_C`'s.
- **Step 2 (sticky-mark generational), for the lead.** This PR takes the flags byte's last bit. That does not block step 2, and this is why:
  - **Old/young needs no header bit in a sticky-mark design.** Marks already live in the side bitmaps (docs/core.md §10). A sticky-mark collector keeps an old object's mark across minor collections instead of clearing it, so "old" is the kept mark, read from the bitmap.
  - **The barrier needs no new bit either.** `BARRIER` was reserved for exactly this (docs/core.md §12 item 8): a collector sets it on the objects it promotes, and the first store into one logs it. So the fast path stays one test of the flags byte, against `BARRIER | FROZEN`. Step 2 adds nothing to it.
  - **If step 2 wants the value's age in the slow path** (GNU R's `CHECK_OLD_TO_NEW` tests the value too, so that an old-to-old store is not logged), the age comes from the value's mark bit in the side bitmap: a block mask, a bit index and a load, about 5 instructions, in the slow path only. A header bit from `extra` would need one free in every type's map (its maps list their free bits). It would cost about 2 instructions, in the slow path only, and needs no change to the fast path. Neither touches the fast path.
  - **`FROZEN` can be read as "the permanent old generation"** (#948's amendment 1 already says the frozen set becomes the oldest generation's permanent part). A frozen object is old, is never traced by a minor or a major collection, and is reclaimed only by this pass, which becomes the frozen part's "major" collection. So step 2 reuses `FROZEN` and its pass, and does not compete with them for the bit. The one thing step 2 must keep is the I2 check of (d), extended to its own generations.
  - This paragraph goes to the lead with the generational decision, as written, and does not hold this PR.
- **Noted refinements, not in this PR:** an exemption for writes over a pinned environment, which would remove most promise-forcing events, and a one-hop test for a new chain head. "Freeze at force" removes most of the promise-forcing cost later.
- **Rejected alternatives:**
  - a bitless test (`is_frozen_at`'s binary search, about 15 compares), because the old-value test runs on every store into a frozen object;
  - a separate "logged" bit. With `FROZEN` in the fast-path mask, `BARRIER` keeps its meaning, "not yet logged", so no third bit is needed.
- **Test, before the build:** a grep of every reader and writer of the flags byte (`flags()`, `set_flags`, `OFF_FLAGS`, `has_flag`/`set_flag_raw` with `SPARE`) and every copy of a header (duplicate, shallow duplicate, unserialize, the mover). Each must mask named bits or write a fresh header, so that a copy of a frozen object is never `FROZEN`.

### (f) The bar

- **#948's (e), unchanged:**
  - **Cost:** `young.R` and the explicit `gc()` under the rdb boot within 10% of the interim loader's.
  - **RSS:** the peak-RSS rows unmoved.
  - **Perf gate:** rt-bars, 0 failures.
  - **The check:** clean in every mode.
  - **Miri:** on robigo-heap's listed modules and robigo-rtcore, with the new heap test.
  - **The full `scripts/rt-ci.py`:** run, and sol-loop to APPROVE on the final head.
- **Case (d) is ratcheted.** `freeze-garbage-detach.R` matches GNU R 4.6.2: the detach rewrites the search path over frozen Autoloads, the pass thaws Autoloads and its table, and the environment is finalized at `gc()`. The INTERIM(H) notes in `freeze.rs` and in `options.rs`'s `set_option` are removed, and this entry closes #959's.
- **The other three frozen-garbage cases still match,** and two are added:
  - a base binding whose frozen old value holds a finalizable environment through a frozen object (H.md §26's `.userHooksEnv` shape);
  - `reg.finalizer` on a frozen environment that base then drops;
  - `parent.env<-` on a boot environment off the search path, the old parent then unreachable, and a finalizer on something only it held (the first row of (b) for `set_enclos` on a frozen environment).
- **Dirty events constant in N on every perf row,** under both loaders, from the scratch probe rerun on the final head.
- **The slow path** is at most 40 instructions per store, measured.
- **The pass's cost row** is reported: within 10% of main-before-freeze.
- **The in-place `SetOption` port is unblocked by this PR,** and B does it after. With the pass, a removed or replaced option's frozen cell and value are thawed and collected, as in GNU R, so cases (a) and (b) of #959 keep matching. B's PR measures the restore loop in (c).

### (h) What one walk thaws for a queued container (the foreman's condition on 5)

The scratch probe ran the pass's walk at every unfreeze, with the queued objects left out, and counted the frozen objects it no longer reached and their traced words.
- **In practice: no boot namespace or freeze root of size is queued.**
  - Over the conformance (tests/rt 536 processes, tests/r 696), under the interim loader: 0 unfreeze events. Under the rdb boot: one per slice, an external pointer that is itself a root (`EXTPTRSXP`), thawing 161 objects (731 words).
  - The fastr corpus's shard 1/8 under both loaders (4,001 cases each), and the 60 perf rows at N and 2N under both loaders: 0.
- **robigo-rt freezes no namespace yet but base's.** `baseenv()` and the base namespace are immortal and not frozen, so a weak reference keyed on them queues nothing and never dies, as in GNU R. The compiler and the default packages load after the freeze today (robigo-rt has not yet ported loading them at boot: L5). So `reg.finalizer(asNamespace("cli"), …)` in a package's `.onLoad` (cli's `onload.R`) is on an ordinary environment.
  - Among GNU R's own packages, only `parallel` (an environment of its own, `.fin.env`) and `methods`' reference classes (user objects) register finalizers, and neither on a namespace.
  - Of the 20 pinned packages, cli's is the only one on a namespace (its own), and R6, rlang and cli register them on objects and `defer` scopes.
- **The cliff, made deliberately** (the rdb boot; these are the frozen containers R code can name):

  | `reg.finalizer(x, f, onexit = TRUE)` on | thawed | words |
  |---|---|---|
  | base's S3 methods table (`.__S3MethodsTable__.`) | 1,744 | 12,555 |
  | `.GenericArgsEnv` | 1,375 | 6,496 |
  | `.ArgsEnv` | 976 | 4,520 |
  | Autoloads (`.AutoloadEnv`) | 139 | 626 |

  Under the interim loader only Autoloads is frozen of these, with 5 objects. The frozen heap is 20,836 objects, so the worst case today is 8% of it, traced at every collection from then on.
- **So it is the foreman's (i):** this does not happen to boot namespaces or roots in practice. The PR adds a row, reported and not gated: `young.R` after `reg.finalizer(base's S3 methods table)`, with its collector total against the same script without the finalizer. That keeps the cliff visible. Option (ii), keeping a queued root walkable, is not built.
- **Due now for the compiler (#942, main 894f1896a).** Under the rdb boot the compiler's namespace is frozen: 42,395 frozen objects (2.48 MB), up from 20,836. The counts in this section and in (b)'s promise-forcing row were taken without it. **Retake them on this PR's head:**
  - the unfreeze events by kind, and the words one walk thaws for each, including any finalizer or weak reference on the compiler's namespace or on what only it reaches, over the conformance and the pinned packages;
  - the cliff table, with a row for `asNamespace("compiler")`;
  - the forced-promise dirty counts, which now include the compiler's lazy-load promises.
- **Condition, owner H: re-measure when the compiler and the default packages are frozen at boot** (L5's boot work). Then `reg.finalizer(asNamespace(...))` does hit frozen environments: cli's, in the pinned set, does once cli loads at boot. The count of unfreeze events by kind, and the words one walk thaws for each, is taken again over the conformance and the package gate (once that runs robigo-rt), before L5's change merges. If a frozen namespace or root is queued in practice, option (ii) is designed then.

### (g) Premises, each tested before the build

1. **(b)'s table is complete:** the three greps it names (the no-pointer writers' callers, `CGlobals`' writers after the freeze, raw writes), with #948's premise 2 grep rerun.
2. **The bit is free:** (e)'s grep.
3. **The pass's walk is the freeze's walk:** in a scratch build, run the pass at the freeze point. It thaws 0 objects under both loaders. If it thaws any, the two walks differ, and they must become one function.
4. **The chain test in `set_enclos` is exact for the search path:** the attach/detach loop sets dirty 0 times per iteration when it attaches a user list, and once per detach of a boot-attached package. Measured with the scratch probe.

**Run, 2026-11-21, on scratch builds of #959's head** (`H.md` §29; nothing committed; the probe sets `SPARE` on frozen objects and counts at the store paths, `set_enclos`, `Globals::remove`, the base-binding write, the collection's start and the unfreeze; output in `~/.cache/robigo-research/h-pass-probe/`):
1. **The table's greps:**
   - Every no-pointer writer runs on a word with no traced pointer: `store_scalar` and `unbind` store NULL through `store_val` first, and an unforced PRVALUE is `R_UnboundValue`.
   - The dynamic counts found only the kinds the table names.
   - *Still to do in the build PR's first commit:* the `CGlobals` writers after the freeze, and #948's raw-write grep rerun on the PR's base.
2. **The bit:** `SPARE`'s only user is the mover's `OLD`, which cannot start after the freeze and tests `IMMORTAL` first. `OBJECT` and `LEVELS` read their own bits, and the probe ran the conformance with `SPARE` set on every frozen object: 531/534 and 691/694 under both loaders, the three failures each unratcheted, the same as without it. *Still to do:* the header-copy grep.
3. **The pass's walk is the freeze's walk:** **holds.** Run at the freeze point, before the freeze's referrer thaw, it reaches every frozen object under both loaders. After that thaw (which the one walk removes), the rdb boot has 134 unreached: the thaw's artifact, as (a) says.
4. **The chain test, as first drafted: failed.** `attach`'s write order (the global environment's ENCLOS first, then the new environment's parent) makes a per-write test see a loss twice per attach. It is replaced by the deferred search-path test in (b), which **holds**: the attach/detach loop's dirty collections are constant in N, and the one loss is the detach of Autoloads. Over the conformance with the deferred test, under both loaders and in both slices:
- the path was checked at about 1,330–1,430 collections per slice (those that followed a `set_enclos`), and lost a frozen member at exactly one: the detach case;
- the dirty collections are unchanged from the first run: 27/2,353 and 46/2,458 under the interim loader, 48/2,347 and 79/2,447 under the rdb boot. The per-write rule's false events had all fallen on collections that were dirty anyway;
- the probe build passes 534/537 and 694/697 (interim loader) and 531/534 and 691/694 (rdb boot). The three failures in each are the branch's own unratcheted ones.


### As built (2026-11-23), and what it measured

Built on main 0007ddf33 (#959 merged: the lazy freeze, off by default). The freeze stays off by default here.

**What differs from the plan above:**
- **The pass is not folded into the collection's marking.** That is the next PR (the foreman's step 2), for the first-collection clause.
- **The pass's walk marks what it reaches in the mark bitmap** (a large object in a set), and logs as it walks. There is no hash set and no second trace of the frozen set: one pass under the rdb boot with the JIT on costs about 9.5M instructions, not 17M.
- **`Trace::trace_search_path`, a derive key beside `freeze`,** is the search path's one walk, which the freeze roots call, so the deferred test and the roots cannot differ.
- **Review found these, all fixed with tests** (Sol r1–r4: S1–S13):
  - a frozen object made immortal is pinned, never thawed (a symbol's PRINTNAME: S1);
  - an object made immortal while it holds a frozen one is logged, and pinning a frozen container notes dirty (S8);
  - writable `DATAPTR` reaches the core for any frozen vector, which is marked `SEEN_BY_C` (S2);
  - stopping the mover clears its `OLD` bit (S3);
  - `set_std_len`, `set_promise_unboxed`, `retire_arg` and an alias's clear note the frozen words they cut raw (S4, S9, S12);
  - a collection whose roots cannot decide (no freeze roots, no search path) consumes nothing (S5, S11);
  - I3 compares sets and runs with an empty list, and I2 holds only after a pass step that decided (S6, S10).

**Behaviour (the flip's condition on R's view):**
- `tests/rt/memory/freeze-garbage-detach.R` matches GNU R 4.6.2 with the freeze on, and is ratcheted.
- A new case, `freeze-garbage-finalizer.R` (a finalizer on frozen Autoloads, which base then drops), is recorded and ratcheted.
- All five frozen-garbage cases match with the freeze off, on, and on with the check at every collection, under both boots.
- The rt conformance with the freeze on and the check at every collection, under both boots: no I2 or I3 failure.

**The check's cost under the rdb boot:** `tests/rt/stress/finalizer-depths.R` exceeds the 600 s case limit with the check at every collection, because I3 scans every live object. It takes 6 s at every 16th collection, 13 s at every 4th, and 4 s with no check. A CI step under the rdb boot can afford an interval of about 4 to 16. E3 checks every 16th; the nightly checks every collection under the interim loader only.

**Amended (e), freeze on, against main 0007ddf33** (callgrind instructions; memory the median of 21 interleaved runs):

| clause | interim | rdb, JIT on | rdb, JIT off |
|---|---|---|---|
| startup instructions (within 1%) | −0.48% | +0.06% | −0.04% |
| startup anonymous memory (within 1%) | +1.0% | +0.2% | +0.2% |
| per collection after the first (within 10% of 2.45M, so 2.70M) | 2.33M: holds | **4.20M: fails** | **3.00M: fails** |
| the first collection (at most 2× main's) | **3.8×** (2.97M / 0.78M) | **4.2×** (21.44M / 5.06M) | **3.9×** (10.92M / 2.78M) |

- **Why per collection fails:** `young.R` runs one pass, about 9.5M under the rdb boot, and the clause's average carries it. Two first-use events dirty it:
  - forcing base's lazy-load promises clears PRENV over the frozen lazy-load environment (rdb only);
  - a temporary global handle that held a frozen builtin is released during the first JIT compile (both boots).
- **Why the first collection fails:** the collection plus the freeze's own walk. Removing the thaw (39.4M) and the sort (6.05M) took it from 10–13× to about 4×.

**Next (the foreman's step 2, its own PR):**
- the freeze folded into the first collection's marking, as phase 1, with the ordinary mark loop untouched;
- passes made rarer: a global handle added since the last pass needs no note on release, and base's lazy-load environment may become a freeze root;
- passes made cheaper: a direct visitor.

The default flips there, when all four clauses hold on the three boots.

**X's row `guard-vec-arith`** (one collection in its window, the first after the boot: the freezing one with the freeze on). Instructions per iteration, against main 0007ddf33 with the freeze off:

| boot | main, freeze off | #959, freeze on | this PR, freeze on |
|---|---|---|---|
| interim loader | 88,186 | 89,951 (+2.00%) | 88,736 (+0.62%) |
| rdb, JIT on | 89,246 | 103,776 (+16.28%) | 93,344 (+4.59%) |
| rdb, JIT off | 89,552 | 96,622 (+7.90%) | 91,600 (+2.29%) |

What remains is the freeze's own walk in that collection, which the fold removes. #959's extra cost was mostly `thaw_with_referrers`: it built a reverse-edge map over the whole frozen set, a cost of the set's size, to thaw the few frozen referrers of weak keys and external data (about 20 under the rdb boot). It was not the freeze undoing itself.

**The three starts, this PR against main dff5cef65,** by X's `scripts/rt-startup.py` (#978): each binary boots from the trees its own build writes, in a scratch directory. Instructions are callgrind's, over the heap shifts; memory is the median:

| boot | instructions | anonymous peak (`RssAnon`) | total peak RSS (`VmHWM`) |
|---|---|---|---|
| interim loader | 15,276,122 → 15,311,164 (+0.23%) | 1,880 → 1,884 KiB (+0.2%) | 10,156 → 9,824 KiB (−3.3%) |
| rdb, JIT on | 92,449,565 → 92,342,593 (−0.12%) | 6,944 → 6,952 KiB (+0.1%) | 15,856 → 15,724 KiB (−0.8%) |
| rdb, JIT off | 49,970,316 → 49,796,054 (−0.35%) | 4,684 → 4,688 KiB (+0.1%) | 13,596 → 13,396 KiB (−1.5%) |

With `ROBIGO_GC_FREEZE=1` the three are the same within 0.02% (the boundary only marks blocks). No number moves more than +2%.

A first measurement by `/proc` from inside R, against main 0007ddf33 with the checkouts' own trees, read the interim loader's total peak as +4.3%. `rt-startup.py`'s exact reading above supersedes it: the difference was the trees and the method, not this PR's code.

**The gate on this PR's head** (f58cc9a86, before this entry):
- the full `scripts/rt-ci.py`: exit 0, every step, 62.1 min, the rdb crosscheck comparing 98,890;
- `--shard debug-assertions`: exit 0, 7.6 min;
- Miri on robigo-heap (`layout::`, `heap::bitmap_tests`, `heap::medium_tests`, `arena::`, `gc::freeze`: 29 tests) and robigo-rtcore: clean.

### A start's count follows the heap's layout (CI on 4f420c6e5, 2026-11-23)

perf-rt failed the interim start at +0.54% (15,470,137 against main's 15,386,628) with the freeze off. Measured in CI's oracle image on CI's own two binaries, each start's count is bimodal in the length of its R_HOME path: over lengths 8 to 78, main's ranges over 15,305k to 15,391k and this PR's over 15,329k to 15,428k, each with steps of about 80k at different lengths. CI's paths put this PR on its upper step and main on its lower. Off both steps, the PR was +24k (+0.16%): `make_immortal`'s new frozen-child check, which traced the fields of every symbol a start installs.

The step was in neither the freeze nor the size of a long-lived struct: it was `Wide::parse` (`robigo-rt/src/base.rs`, main's code). Its 1,193 names (about 85 KB) and their `HashMap` grew by doubling, so whether each realloc extended in place or copied depended on what the heap held above it, and the paths' lengths moved that. Main's interim start moved by −92,793 between dc256de9f and dff5cef65 the same way, and C1's #979 by +80,848: the same kind of step, from something else growing.

The fix allocates each of the wide base's tables once, at sizes a unit test checks against `proto/base-wide.txt` (`WIDE_SIZES`). It also skips the frozen-child check while nothing is frozen. On the host, over R_HOME lengths 143 to 213, main is 15,385k to 15,450k, this PR before the fix was 15,277k to 15,473k, and after the fix it is 14,738k to 14,901k (−3.3% to −4.4%). The `HashMap`'s SipHash rehashes were about 280k of that, and the copies about 250k.

**For the next person:** a start's instruction count can step by about 80k (0.5% of the interim start) when anything changes what the heap holds before a large table grows by doubling: a path's length, the environment's size, or a long-lived struct's size class. `rt-startup.py`'s environment shifts do not move R_HOME's path, so they do not show it. Allocate start-time tables at their final size, and when a start moves by about 80k, look first at reallocs and memcpy (`callgrind_annotate --inclusive=yes`, then the deepest function carrying the step).
