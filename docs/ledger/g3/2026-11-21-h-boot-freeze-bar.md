## 2026-11-21: the boot-heap freeze's bar (step 1 of making a collection's cost independent of the old heap)

This was written before any code, for the foreman's signature in the pull request that adds it. Once a run cites it, it is fixed. A change is a new dated entry that names this one and says why, committed before the runs it governs (§15.2).

### The problem, measured

robigo-rt collects every generation at every collection (`Core::collect_needing`), so a collection traces the whole live heap. Measured with callgrind on release `robigo-rt` (main `5a5ce9766`):

| | interim loader | rdb boot | rdb + 100k live calls | rdb + 300k |
|---|---|---|---|---|
| one explicit `gc()`, nothing allocated between | 0.59M | 1.84M | 35.1M | 97.0M |
| collector total for one fixed young allocation (`young.R`: 20,000 × `y <- x * 2 + x`) | 12.3M (5 collections) | 19.3M (5) | 266M (7) | – |

The rdb boot alone triples a collection, because base's byte code and the compiler are traced every time. GNU R's level-0 collection marks only from the roots and the old-to-young lists. This step removes the boot heap from every later collection. Step 2, sticky-mark generational collection, is the lead's decision and is not covered here.

### (a) What is frozen, and when

- **When.** Once, at the end of the boot: after `setup_Rmainloop`'s last step and before the first expression of the session or script runs.
  - Under the interim loader that is after `stage0` (base's files, the system profile, `lock_base`) and `after_profile`: `.OptRequireMethods`, the site and user profiles, `.First.sys`, and the JIT setup that loads the compiler at startup when the JIT is on (`l5`: `loadedNamespaces()` is `c("compiler", "base")`).
  - Under the rdb boot it is the same point, after `..lazyLoad` has bound base from `base.rdb`.
  - Either way, the freeze runs after one full collection made at that point, so only the boot's survivors are frozen.
  - **With default packages** (amendment 4): the freeze comes after `.First.sys`, so a start that loads the default packages there (`R_DEFAULT_PACKAGES` unset) freezes them too, with their namespaces and attached package environments. The rule is the same: everything reachable at that point from the roots listed below. Premise 1's 90% and premise 6's size are measured for the `R_DEFAULT_PACKAGES=NULL` start only, until the default packages load under robigo-rt. Then both are measured for that start as well, and recorded beside the NULL start's.
- **What.** Every object reachable at that point from the freeze roots, made immortal (`Core::make_immortal`, with `BARRIER` set). The freeze roots are (decided 2026-11-21 on premise 1):
  - the base environment and the base namespace;
  - the symbols' base cells (`SymCells`);
  - the namespace registry, and each namespace loaded by then (the compiler's), with their frames and hash tables;
  - robigo-rt's `Globals`: the Rust-held handles of the dispatch and method caches, the JIT cache, the fold constants, the boot's stage-0 objects, the ALTREP classes, the loaded DLLs' info, the connections and the condition token;
  - the C API's constants and symbols (`capi.rs`).
- **The containers written after the boot** (decision 1), and how each is handled:
  - **left out by rule:** the global environment and its frame; `.Random.seed`, which lives there;
  - **not heap values held by a freeze root:** the warnings (robigo-rtcore's `Warnings` root), the handler and restart stacks (context state), and `.Last.value`, `.Traceback` and `.Options` as bindings (each a `SymCells` value, a Rust root: replacing the binding stores nothing into a frozen object);
  - **frozen, and logged as a root at the first store:**
    - `.Options`' pairlist cells: `options()` writes a cell's CAR, which logs that cell once;
    - the search path's package environments' ENCLOS words, written when a package attaches (rare);
    - the namespace registry's table, written when a namespace loads;
    - base's BCODESXPs and closures, which the JIT caches code on (`code::cache_on`, `set_aux`), once each;
    - base's lazy-load promises, which forcing writes once each.

    None of these is written by every script on a hot path. Measured (premise 7's survivor bit, a store counter): over every perf row at N and 2N under the rdb boot, no row stores into a boot survivor per iteration. The counts are constant in N: from 5 to 21 for most rows, 3,318 for `jit-safe`, all one-time stores of the kinds listed. So the wider root set puts no frozen object on a hot store path (decision 2).
- **What is not frozen:**
  - the global environment's frame and what only it reaches: what the user's profile made is the user's data;
  - weak references, and their keys and values where only a weak reference reaches them: their death is observable;
  - objects with a finalizer (`reg.finalizer`, an external pointer's), for the same reason;
  - external pointers whose address C can still free (the heap's release callbacks);
  - objects C holds a payload pointer to (`SEEN_BY_C`, with `DATAPTR` of a STRSXP or VECSXP): C can write their elements without a barrier;
  - anything a frozen object reaches only through one of these. The traversal stops at each, and the stopped-at object stays unfrozen.

  The objects left out stay ordinary and are traced as now. A frozen object that points to one of them is logged as a root at the freeze (premise 3).

### (b) The invariant, and every store path into a frozen object

**Invariant.** At every collection, every unfrozen object that a frozen object points to is reachable from a root. A frozen object written since the freeze is in its heap's remembered set, which is a root. Frozen objects are never traced except as remembered roots.

It holds by construction if every store of an object pointer into a frozen object passes the barrier (`Sexp::store`, `store_uncounted`: the `BARRIER` flag's slow path logs the object as a root). The paths, each with its barrier:

| path | the store | barrier |
|---|---|---|
| R-level assignment in base's environments (`<-` at top level of base, `assign(, envir = baseenv())`) | the symbol's base cell | none needed: `SymCells` are Rust tables traced as roots at every collection (`sym.rs`, `Trace`) |
| the base namespace's bindings | the same cells | as above |
| assignment in a frozen hashed environment (the compiler's namespace) | a binding cell's CAR (`chain::set_value` → `set_car`), a new cell (`set_hashtab`'s chain head, `set_elt`), the table grown or rehashed (`hashtab::resize` → `set_hashtab`, `set_elt`) | `Sexp::store` in `set_car`, `set_elt`, `set_hashtab` |
| assignment in a frozen slot frame (a closure environment base made) | `SlotRef::store_val`, `store_aux`, `store_val_uncounted` (`env/frame/layout.rs`), frame words (`layout::set`) | `Sexp::store` / `store_uncounted`. `set_key` writes a symbol (immortal: no barrier needed) and `set_val_bits` an unboxed scalar (no pointer) |
| a base promise forced (PRVALUE written, PRCODE and PRENV cleared) | `set_promise_value`, `set_cdr`, `set_tag` | `Sexp::store`. `set_promise_unboxed` writes no pointer |
| `assignInNamespace`, `unlockBinding`, `lockBinding`, active bindings (`makeActiveBinding` in base) | a cell's value and gp bits | the value through `set_car` (`store`); gp bits are not pointers |
| attribute writes on a frozen object (`attr<-`, `class<-` on a base object, `SET_ATTRIB`) | `set_attrib` | `Sexp::store` |
| the C API: `SETCAR`, `SETCDR`, `SET_TAG`, `SET_ATTRIB`, `SET_VECTOR_ELT`, `SET_STRING_ELT`, `SET_PRVALUE`, `SET_FORMALS`, `SET_BODY`, `SET_CLOENV`, `SET_ENCLOS`, `R_SetExternalPtrProtected`, `R_SetExternalPtrTag` | the matching heap setters | each through `Sexp::store` (no raw setter outside robigo-heap: `set_*_raw` has no caller in rtcore, rt, rtbuiltins or rtapi, checked 2026-11-21) |
| C writing an element through `DATAPTR`, `STRING_PTR_RO` cast away, `VECTOR_PTR` | a raw word | **none.** Such objects are `SEEN_BY_C` and not frozen (above), and an object that becomes `SEEN_BY_C` after the freeze is a frozen object getting a pointer handed to C: premise 4 |
| C's first writable `DATAPTR` of a frozen STRSXP, VECSXP or EXPRSXP (premise 4's hardening) | the object's elements, raw, from then on | `DATAPTR` logs the object as a root once (a flag test on the call). It stays immortal, and is traced at every collection from then on, so raw pointer writes into it are seen |
| a code object cached on a BCODESXP (C1's #943: `code::cache_on`, the body's 4th word, and the code object's `OTHER` element) | `set_aux`, `set_elt` | `Sexp::store`: a heap store through the barrier (`crates/robigo-rt/src/code.rs:632`) |
| a closure's cached code (`compile`'s code on the closure's 4th word; the JIT's) | `set_aux` | `Sexp::store` |
| vectors filled in place (`FreshVec::put`'s raw element write, `crates/robigo-rtcore/src/fresh.rs:621`) | a raw word | only on a fresh, unpublished vector, which is never frozen |
| the global cache and `R_GlobalCache` entries | `SymCell::cache` | Rust tables, roots |

Premise 2 below has the test that this table is complete.

### (c) The debug mode that proves it at every collection

`ROBIGO_GC_FREEZE_CHECK=1`: at every collection, before sweeping, a full trace that ignores `IMMORTAL` for the frozen objects marks every reachable object into a second bitmap. The run aborts, naming the object and the frozen object that reaches it, if an object that bitmap marks is unmarked by the frozen-aware trace. rt-ci runs it:
- with gctorture (collect at every allocation);
- with `ROBIGO_GC_MOVE=1`;
- with `ROBIGO_GC_MOVE=young:16`;

each over the conformance slice and robigo-rtcore's and robigo-heap's tests.

**The moving collector** (amendment 2):
- A frozen object is immortal, and an immortal object never moves. With the freeze on, `ROBIGO_GC_MOVE=1` and `=young:N` would move only what the session made after the boot, and so test less: the move modes exist to find stale raw pointers across safepoints, and most raw pointers held at a safepoint point into base's code.
- So **the freeze is off under both move modes:** the boot heap stays ordinary and moves as today.
- The frozen-aware path is covered by `ROBIGO_GC_FREEZE_CHECK=1`, under gctorture without moving, and in the release tests.
- The production collector does not move, so the freeze costs no compaction. A frozen object's referents need nothing special: the remembered set's entries are traced as roots, as an immortal object's are today.

### (d) What R can see

- **`gc()`:** used cells and Mb drop by the frozen heap, unless the counts keep it. The L7 draft (#946) makes totals robigo's own and keeps exact only the shape, limits, `reset` and max-used semantics, trigger rules, report format and retained-data deltas. A freeze keeps all of those. If the lead does not sign #946, `gc()`'s totals must still count the frozen objects (`immortal` counts are kept per type today), and the bar adds that `gc()`'s used columns are unchanged by the freeze.
- **`memory.profile()`:** counts by type include the frozen objects, which are still in the heap's blocks. The freeze changes no count.
- **`object.size`:** unchanged; it reads the object, not its mark state.
- **`reg.finalizer` on a base object after the freeze** (an environment of base's). The object is frozen, so it would never die and its finalizer would run only at exit with `onexit = TRUE`. GNU R would also never collect a base environment that base still reaches, so the finalizer behavior matches. A finalizer registered on a frozen object that base later drops (an environment that only a replaced binding reached) would never run in robigo, and would run in GNU R after the next full collection. That is an observable difference. The bar requires a case for it, and the freeze either unfreezes an object at `reg.finalizer` (it is logged and re-traced from then on) or the difference is documented. Decided 2026-11-21 (premise 5): unfreeze, batched. **No object is ever unfrozen between collections.**
  - **Events:** `reg.finalizer` on a frozen object; a weak reference keyed on one (`R_MakeWeakRef`, `new.env`'s finalizers, `reg.finalizer`'s weak reference); a C API `R_RegisterCFinalizer*` on one. Each puts the object on a pending list and returns; nothing can free it before a collection anyway.
  - **At the next collection,** if the list is not empty, one pass over the frozen heap logs as a root every frozen object that refers to a pending one. Then it clears `IMMORTAL` on the pending objects, and the collection proceeds. One pass covers the whole list: 10,000 finalizers registered on base's environments cost one pass.
  - **Invariant:** every frozen referrer of an unfrozen object is a logged root. The unfrozen object's own references to frozen objects need nothing (a frozen object is never freed).
  - **The unfrozen object's children that were frozen stay frozen.** Only the pending objects change.
  - **`gc()`'s counts:** an unfrozen object moves from the immortal counts to the ordinary ones, and is counted where it would have been without the freeze.
  - **The check mode** covers it: after an unfreeze pass, the debug traversal (c) verifies that no frozen object that is not logged refers to an ordinary one.
  - **Oracle cases** (GNU R 4.6.2):
    - a finalizer on a boot object whose last reference is then dropped (an environment base held, released by `rm` of an unlocked base binding), then `gc()` and the finalizer's output;
    - a weak reference keyed on a boot object, the key dropped, `gc()`, and the weak reference's value. A finalizer registered on a base environment or namespace itself (`reg.finalizer(baseenv(), f, onexit = TRUE)`, or on `asNamespace("compiler")`): the environment is unfrozen and stays reachable from its root, so it never dies, and the finalizer runs only at exit with `onexit = TRUE`, as in GNU R. A case pins this.
- **A base function replaced after the freeze** (`assignInNamespace("f", g, "base")`, or any reassignment in base). The old value stays frozen and is never freed. The leak is bounded by what the boot made: at most the frozen heap's own size (measured by premise 6 before the build), whatever the number of reassignments, because a frozen object is never copied. A session that reassigns base repeatedly leaks only the first replaced value of each binding; later values are unfrozen and collected as now.

### (e) The bar

- **Cost:** with `young.R` under the rdb boot, the collector's total within 10% of the interim loader's (12.3M today), and an explicit `gc()` under the rdb boot within 10% of the interim loader's 0.59M.
- **RSS:** the rt-bars peak-RSS rows unmoved (2.0% against main), under both loaders.
- **Perf gate:** rt-bars, 0 failures, under the interim loader against main. The same rows under the rdb boot are reported against main's rdb boot, and every guard row's collector share is not higher.
- **The check:** `ROBIGO_GC_FREEZE_CHECK=1` clean in every mode listed in (c).
- **Miri** on robigo-heap (`layout::`, `heap::bitmap_tests`, `heap::medium_tests`, `arena::`) and robigo-rtcore, with a heap-level test of the freeze, its barrier log and its check.
- **The full `scripts/rt-ci.py`;** sol-loop to APPROVE on the final head.

### (f) Premises that could fail, and each one's test before the build

1. **The frozen set is most of the boot's live heap.** If base's lazy-load promises (unforced) or the compiler's structures are mostly reachable only from the global environment, or are not reached at all, freezing saves little. Test: at the freeze point, count the objects and bytes reachable from the roots in (a) against the whole live heap (a debug traversal on main's binary, both loaders). Proceed if at least 90% of the live heap's traced words are in the set.
2. **The store table in (b) is complete.** Test:
   - a grep of robigo-heap's public API for every function that writes an object pointer into an existing object, classified as through `store`/`store_uncounted` or not (2026-11-21: `set_attrib`, `set_car`, `set_cdr`, `set_tag`, `set_elt`, `set_aux`, `set_promise_value`, `set_enclos`, `set_frame` and `set_hashtab` go through `store`; `set_promise_unboxed`, `set_extptr_addr`, `set_val_bits`, `set_key` write no pointer or an immortal one);
   - a grep of rtcore, rt, rtbuiltins and rtapi for raw pointer writes (`*(... as *mut Sexp) =`, `ptr::write`, `copy_nonoverlapping` into an object's body). Each hit is in the table or on a fresh object.
3. **The objects left out are found in one pass.** Test: weak references, finalizable objects, `SEEN_BY_C` objects and external pointers with release callbacks are each identifiable from the header or the heap's lists at the freeze (`Core`'s weak and finalizer lists, `extra::SEEN_BY_C`), with no search of the heap.
4. **A frozen object never gets a payload pointer handed to C** (`DATAPTR` of a frozen STRSXP or VECSXP from a package's C after the freeze). If it can, C writes past the barrier. Test: the C API's `DATAPTR`, `STRING_PTR`, `VECTOR_PTR`, `STRING_PTR_RO` and `DATAPTR_RO` entries. Each mutable one either unfreezes its object first (logs it and clears `IMMORTAL`, as `SEEN_BY_C` does today for moves), or the bar names it.
5. **Unfreezing an object is cheap and sound** (premise 4's and `reg.finalizer`'s case). Test: a design note showing that clearing `IMMORTAL` and logging the object restores ordinary tracing, before any code. A frozen object's children are frozen too, and they stay frozen and reachable from the now-traced object, so the invariant holds.
6. **The leak in (d) is as stated.** Test: the frozen heap's size at the freeze point under the rdb boot (Ncells, Vcells, bytes), recorded in the PR's entry.
7. **In-place writes stay in place** (amendment 3). Immortal objects skip reference counts, so a frozen object's `REFCNT` would freeze at its value at the freeze. A count frozen at 1 would let `y <- base_value; y[1] <- 0` modify base's copy. So a frozen object is treated as shared (`MAYBE_SHARED` true), and every modify of a value held by base or a boot namespace copies. That must not move a steady-state path. What base and the compiler modify in place today, and where each stands:
   - **`.Options`:** a pairlist in base. `options()` replaces cells' values through `SETCAR` (`set_car`, the barrier), and does not copy-on-modify a vector. Its cells stay frozen; a changed cell is logged once.
   - **`.Random.seed`:** in the global environment, not frozen.
   - **The compiler's state:** environments (`compiler:::compilerOptions` and the like) assigned through bindings, not vectors modified in place. Bindings are not copy-on-modify.
   - **Condition handler stacks and restarts:** robigo-rtcore's context state (Rust), not heap values held by base.
   - **`.Traceback`, `.Last.value`:** bindings replaced, not modified.
   - **Anything a perf row touches:** the rows' own data lives in the global environment or in the row's frames, not frozen. A row that reads a base vector and modifies a copy already copies today.

   The test, before the build:
   - a debug counter of copies made because `MAYBE_SHARED` is true for an immortal object, run over every rt-bars row under the rdb boot with the freeze on; every row reads 0 copies, or each copy is listed and justified;
   - with the freeze, the rows under the rdb boot are part of the bar (e). A row that starts copying is a failure, not an accepted cost.

   Also: list every reader of `flags::IMMORTAL` and of the refcount skip (the C API's `MAYBE_SHARED`, `mark_not_mutable`, the move modes' pinning), and show that each is right for a frozen object.

### The premises, run (2026-11-21, before any build)

These were measured with scratch builds that were never committed (`H.md` §25).
1. **Frozen share of the live heap** (`--vanilla`, `R_DEFAULT_PACKAGES=NULL`):
   - from base's roots only: 82.0% (rdb boot), 60.0% (interim loader);
   - with `Globals` and the C API's constants: **93.9% (rdb, judged: holds)**, 73.5% (interim loader: recorded, not judged, since that loader is to be deleted).

   The rest is mostly already-immortal objects (PRINTNAMEs, symbols, builtins).
2. **The store table:** holds. Every raw write outside robigo-heap is into a fresh object, an immortal fill, plain data, or a read.
3. **The objects left out are found in one pass:** holds (Core's weak, finalizer and release lists; the `SEEN_BY_C` bit).
4. **C's `DATAPTR`:** holds by GNU R's write-barrier contract, hardened (the store table's row).
5. **Unfreezing:** batched, as decided in (d).
6. **Frozen size:** about 1.7–1.9 MB per isolate under the rdb boot (NULL start), and 0.27 MB under the interim loader. This bounds the leak from replaced base values.
7. **In-place writes:** a survivor bit after a full collection, counted at `Ref::maybe_shared`, `Ref::maybe_referenced` and the VM's in-place path. 70 of 70 perf rows read 0 under the interim loader, and 69 of 70 under the rdb boot. `source5k` is not runnable under the rdb boot for lack of `tempdir()`, which has no owner yet (B's notes call it a trial delegate's).

### Once generational collection exists (amendment 1)

The frozen set becomes the oldest generation's permanent part, and the freeze stays as its boot-time shortcut. Base and the boot's namespaces are not garbage in any session that runs R code: a major collection would trace them only to keep them.
- **Kept from this step:**
  - the store table (b), which is the barrier inventory a generational collector needs anyway;
  - the barrier's logging into the remembered set (the old-to-young check is added to the same slow path);
  - the check mode (c), which becomes the minor-collection verifier: each minor collection's marks compared with a full trace's.
- **What changes:** a major collection could unfreeze (re-trace) the frozen set if a later measurement shows frozen garbage worth reclaiming. That is a policy flag, not a redesign.

### The rows that make the cost visible now (item 3)

These are proposed for A1 to wire. The scripts are `young.R` as above, with `EXTRA` live calls (`as.call(list(as.name("f"), i, i + 0.5))`, five objects each):
- `gc-young-0`: no extra live data;
- `gc-young-100k`: 100k live calls, about a large package's code;
- `gc-young-300k`: 300k.

Each reports instructions per iteration (`perf.py`'s 2N − N method), the collector's share (callgrind: `collect_needing` inclusive over total), and the GNU R reference for the same script (GNU R 4.6.2, by the rows' existing oracle method).

**They should report only, and not gate, until step 2 exists.** Today `gc-young-100k` is about 21 times `gc-young-0` in collector cost by design: a gate would fail every PR, or be set so loose it catches nothing. Once the freeze lands, `gc-young-0` gates (its collector share must not rise). Once step 2 lands, the 100k and 300k rows gate against GNU R's ratio.
