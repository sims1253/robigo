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
- **What.** Every object reachable from the base environment, the base namespace, the symbols' base cells (`SymCells`), the namespace registry and each namespace loaded by then (the compiler's), and their environments' frames and hash tables. They are made immortal (`Core::make_immortal`, with `BARRIER` set).
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

each over the conformance slice and robigo-rtcore's and robigo-heap's tests. The move modes never move a frozen object: it is immortal, and its referents are pinned or forwarded through the remembered set's trace.

### (d) What R can see

- **`gc()`:** used cells and Mb drop by the frozen heap, unless the counts keep it. The L7 draft (#946) makes totals robigo's own and keeps exact only the shape, limits, `reset` and max-used semantics, trigger rules, report format and retained-data deltas. A freeze keeps all of those. If the lead does not sign #946, `gc()`'s totals must still count the frozen objects (`immortal` counts are kept per type today), and the bar adds that `gc()`'s used columns are unchanged by the freeze.
- **`memory.profile()`:** counts by type include the frozen objects, which are still in the heap's blocks. The freeze changes no count.
- **`object.size`:** unchanged; it reads the object, not its mark state.
- **`reg.finalizer` on a base object after the freeze** (an environment of base's). The object is frozen, so it would never die and its finalizer would run only at exit with `onexit = TRUE`. GNU R would also never collect a base environment that base still reaches, so the finalizer behavior matches. A finalizer registered on a frozen object that base later drops (an environment that only a replaced binding reached) would never run in robigo, and would run in GNU R after the next full collection. That is an observable difference. The bar requires a case for it, and the freeze either unfreezes an object at `reg.finalizer` (it is logged and re-traced from then on) or the difference is documented. The default is to unfreeze, and premise 5 tests it.
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
7. **Nothing reads `IMMORTAL` to mean "a shared constant"** in a way the freeze would break (the C API's `MAYBE_SHARED`, refcount skipping, `mark_not_mutable`). Today, immortal objects skip reference counts, so a frozen object's `REFCNT` would freeze too. Test: list every reader of `flags::IMMORTAL` and of the refcount skip. A frozen object whose `REFCNT` matters to copy-on-modify (`MAYBE_SHARED` decides whether `x[i] <- v` copies) must either keep counting, or be treated as shared (always copied). The safe default is shared, which GNU R's `MARK_NOT_MUTABLE` on base's objects already implies for most of them.

### The rows that make the cost visible now (item 3)

These are proposed for A1 to wire. The scripts are `young.R` as above, with `EXTRA` live calls (`as.call(list(as.name("f"), i, i + 0.5))`, five objects each):
- `gc-young-0`: no extra live data;
- `gc-young-100k`: 100k live calls, about a large package's code;
- `gc-young-300k`: 300k.

Each reports instructions per iteration (`perf.py`'s 2N − N method), the collector's share (callgrind: `collect_needing` inclusive over total), and the GNU R reference for the same script (GNU R 4.6.2, by the rows' existing oracle method).

**They should report only, and not gate, until step 2 exists.** Today `gc-young-100k` is about 21 times `gc-young-0` in collector cost by design: a gate would fail every PR, or be set so loose it catches nothing. Once the freeze lands, `gc-young-0` gates (its collector share must not rise). Once step 2 lands, the 100k and 300k rows gate against GNU R's ratio.
