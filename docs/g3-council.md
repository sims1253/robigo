# G3 council record

The record behind `docs/g3.md`: the briefs, each delegate's design or
review per round, the round-1 synthesis and experiment A's results, as
written (paths normalised). Delegates: GPT-6 Astra, Claude Fable 5.2,
Claude Opus 5.5, Muse Wonder Full (GPT-6.2 Sol was requested but not
available). Rounds 1–2 designed; rounds 3–5 reviewed the merged doc.

| Delegate | Round 3 | Round 4 | Round 5 |
| --- | --- | --- | --- |
| GPT-6 Astra | 7/10 | 9/10 | 9/10 |
| Claude Fable 5.2 | 7/10 | 9/10 | 9.5/10 |
| Claude Opus 5.5 | 7/10 | 8.5/10 | 9/10 |
| Muse Wonder Full | 5/10 | 7.5/10 | 8/10 |

The one open point after round 5 (L7, GC report numbers) was decided by the
lead: reproduce them (`docs/g3.md` §3).


## Round 1

### Brief

### Council brief: design G3, robigo's runtime rebuilt on robigo-core

You are one of four independent council members (GPT-6 Astra, Claude Fable 5.2,
Claude Opus 5.5, Muse Wonder Full). Each writes a full design on their own; the
coordinator compares them. Do not try to guess what the others write.

#### Repository and rules

- Repo: `<repo>`, base commit
  `4cff0ceb` (main). robigo reimplements GNU R 4.5.0 in Rust: identical output,
  messages and semantics, and the fastest R there is. Effort and churn are never
  arguments; if a design is faster for more work, it wins.
- **Read-only.** No edits in the repo, no commits, pushes, PR or issue comments,
  no subagents of your own, no builds that write into the repo's target
  (reading code, `rg`, `git log`/`show` are fine; if you want a measurement,
  say what command would settle it instead of running a long build).
- GNU R 4.5.0 sources: `~/.cache/robigo-research/rsrc/R-4.5.0` (the spec;
  `src/main/{memory,envir,eval,context,errors,bytecode}.c`, `include/Defn.h`).
- Write your whole design to the file named at the end of your task message.
  That file is your only deliverable; your final chat reply is just its path
  and a five-line summary.

#### Read first

`AGENTS.md`; `docs/architecture.md`; `docs/design.md` §3, §5 (esp. §5.3 F1–F8,
§5.5, §5.6 G3); `docs/core.md` in full (the core object layout §2, heap §3,
strings §4, unwind §5, the bridge §6, the council table §12: item 24 "G3-facing
safe accessor layer spec (borrowed reads)" and item 25 "start G3" are open);
`docs/ledger/perf-vm.md` (top entry on #523 and the section "For G3: which
techniques carry over to the new core"); `docs/ledger/core.md` (newest entries
first, especially 2026-11-10/11 and "conservative scan limited to C frames");
`docs/ledger/memory.md` (the bake-off). Code: `crates/robigo-core/src/`,
`crates/robigo-capi/src/bridge.rs`, `crates/robigo-runtime/src/{value,env,interp,
vm,compile,jit}.rs`, `crates/robigo-runtime/src/env/`, `vm/`, `jit/`.

#### State (measured by the coordinator at 4cff0ceb)

- Conformance: FastR snippets 31,358/31,374; help-page examples 1,121/1,153;
  conformance scripts 600/622; builtins 712/761; top CRAN packages 99.9% of
  8,693 test blocks, 18/20 green (Rcpp included), 42/42 install from source.
  Speed: geometric mean 2.95x faster than GNU R over the benchmark suite, but
  package suites run at only ~0.7–1.0x GNU R's time.
- G2 is done in substance: robigo-core (mark-region heap, 48-byte nodes with a
  reserved traced 4th word, CHARSXP cache with encodings, unwind with C
  trampolines) and robigo-capi (libR API) work; packages build from source.
  Today the runtime keeps its own `Rc`-based `Value` (16-byte enum of `Rc`s,
  `value.rs:885`) and converts at `.Call` through a ~6k-line bridge with HOST
  proxies for environments/closures/promises and zero-copy EXTDATA vectors.
- Size of the coupling: robigo-runtime ~115k lines (builtins ~60k); ~4,900
  `Value::<Variant>` uses; 142 `Rc::make_mut`; 286 `RString`; 69
  `thread_local!`s.
- Open issues caused by two object models with separate identity (G3 should
  make them impossible): #278 data.table list element identity, #277 C-built
  promise not forced, #276 C writes to purrr's loop index lost, #189 rlang
  env_clone split-brain, #304 zap() identity, #167/#389 freed copies, #217 C
  mutation frees memory the evaluator reads, #460 compact seqs materialized,
  #140 tracemem.
- Where robigo still loses to GNU R: the general closure call (1.1–1.8x
  slower: defaults left to promises, environment methods, `<<-`, S3 dispatch,
  tryCatch, `c(i, 2, 3)`); `benchmarks/suite/calls.R` exercises it.
- The libR.so shape (runtime in a shared lib) was rejected for now because
  Rust thread_locals go through `__tls_get_addr`; revisit after the isolate
  record (core.md §12 item 15).

#### The coordinator's proposed plan (critique it, don't just accept it)

1. A G3 design, council-reviewed. 2. A measured prototype: a minimal evaluator
on robigo-core (closures, envs, promises, scalar arithmetic) vs today's VM on
fib, binarytrees, `benchmarks/suite/calls.R`. 3. Layout-independent prep on
main: narrow `Value`'s interface (private variants behind constructors and a
typed `view()` for matching) so the swap is local to `value.rs`; move
thread_locals into an isolate record. 4. One representation swap on a
long-lived integration branch gated by every ratchet, then delete the bridge.
The coordinator argues against a staged mixed Rc/GC heap (attributes point
anywhere; cross-heap rooting and cycles are unsolved per the core ledger).

#### What to design (all of it; concrete, with types, offsets and file names)

1. **The Rust-side object API** (council item 24): what `Value` becomes (Copy
   `Sexp` newtype? lifetime-branded handles?), typed views for matching and
   for builtins (`RVec<f64>` → slices), borrowed reads and the rules that keep
   them sound against GC, ALTREP materialization and C writes through
   `REAL(x)`; mutation and copy-on-modify through refcounts (NAMED/REFCNT
   semantics vs today's `Rc::make_mut`); how builtins take arguments by value
   for in-place replacement.
2. **Rooting Rust frames:** conservative scan of the whole stack (the bake-off
   assumption) vs precise handle scopes vs a hybrid; interaction with
   deterministic finalizers (why the scan was limited to C frames), with VM
   register files and JIT frames, and with LLVM keeping pointers only in
   registers or derived forms. Pick one and justify with how other systems do
   it (name the files: e.g. V8 handle scopes, JSC conservative scan, Julia
   GC frames, GNU R PROTECT).
3. **Environments:** frame representation (slot frames in the 4th word,
   hashed frames, GNU R FRAME/HASHTAB compatibility for C via the API),
   binding flags, active bindings, locking, version counters for VM guards,
   global cache (GNU R R_GlobalCache), `<<-`.
4. **Closures, promises, argument matching, contexts:** promise layout
   (unboxed scalar values, caller-forced promises), `...`, the context stack
   (RCNTXT for C, sys.function/sys.call/on.exit/restarts), and `Flow` (Rust
   `Result`) vs core unwind across C ↔ Rust ↔ C nesting.
5. **Strings:** CHARSXP replacing `RString`/`RStr`, encodings, string
   vectors, hashing, what builtins see (`&[u8]`/`&str` views, translate
   points), performance of `paste`/`match`/`unique`.
6. **Vectors and ALTREP:** storage variants (inline, compact, foreign,
   ALTREP) in the new world, growth (`x[n+1] <- v` amortized), attributes.
7. **VM and JIT on Sexp:** register file (Sexp or unboxed scalar), what
   carries over from perf-vm (see the ledger list), inline caches, JIT ABI
   against header offsets, deopt, how JIT frames are rooted.
8. **The isolate record:** what moves in, how hot paths reach it (one TLS
   read, a pointer passed down, or `&mut Interp`), interaction with capi's
   per-isolate exports and with the libR.so revisit.
9. **Migration strategy:** the step-by-step order of work with what lands on
   main vs the integration branch, how ratchets stay green, how the
   ~4,900 match sites and ~60k lines of builtins get converted (mechanically?
   codemod? by hand per module?), how long the branch lives, when the bridge
   dies, what work on main should pause now and what continues. Explicitly
   evaluate the coordinator's "no mixed heap" argument and any alternative
   (e.g. a new runtime crate grown beside the old one).
10. **The prototype and measurement plan:** what to build first to settle the
    big decisions by numbers (which decisions, which benchmarks, which
    metrics: callgrind instructions, allocations, peak RSS, pauses), and the
    exit criteria that let the full rebuild start.
11. **Risks and open questions** ranked, with the measurement that resolves
    each.

Ground claims in the code and docs you read (cite `file:line`); mark guesses
as guesses. Be bold: if the coordinator's plan or the existing core design is
wrong for G3, say so and say what replaces it.


### Astra

### G3 design: one object graph, explicit execution state

GPT-6 Astra, independent council member. Read-only review of commit `4cff0ceb142dfa4547a840b1bfa4e1fa4b7360f4`. No builds or measurements were run. Performance thresholds below are proposed acceptance criteria, not results. Repository citations are relative to its root; GNU R citations use `R/src/...` for `~/.cache/robigo-research/rsrc/R-4.5.0/src/...`.

#### Decision

Rebuild execution on the existing non-moving core heap. Use one canonical SEXP for every observable object; preserve unboxed execution values where identity has not escaped. Root Rust execution precisely and keep conservative scanning confined to C segments. Expose borrowed data only during a period that forbids heap mutation, callbacks and collection. Preserve the current VM's semantic optimizations, but rebuild its storage and rooting.

Approve the coordinator's measured prototype and eventual single representation cutover. Reject three stronger claims implicit in the proposal:

1. A private `Value` enum plus `view()` cannot make this change local to `value.rs`. Allocation, liveness, effects, argument ownership and C reentry enter builtin signatures and evaluator control flow. A wrapper can reduce syntax churn; it cannot hide these obligations.
2. The memory bake-off does not establish the cost of precise roots, canonical promises, observable environment bindings or the final 48-byte core layout. Retain the winning collector, but measure the actual execution interface before committing to it.
3. A long-lived branch need not contain all development. Land the new core interface and a separate executable prototype on main while the existing runtime remains the default. No object crosses between them. Keep only the coordinated semantic conversion and cutover on an integration branch.

The most important prototype is the general closure call. Fib already bypasses much of it; the November 10 ledger explicitly records that scalar boxing is a small part of the remaining call cost (`docs/ledger/perf-vm.md:9`). A cheap allocation loop alone would give the wrong answer.

#### 1. Rust object interface and sharing

##### Representation and roots

Retain raw `Sexp` as the unsafe core/C representation. It is currently a public raw pointer (`crates/robigo-core/src/lib.rs:140`); it must not become a safely dereferenceable, freely escaping runtime `Value` by renaming it.

The safe builtin interface uses a direct, lifetime-branded pointer:

```rust
// Proposed core/src/access.rs and roots.rs; schematic signatures.
###[repr(transparent)]
struct Value<'s> {
    raw: Sexp,
    brand: PhantomData<&'s ScopeBrand>,
    single_threaded: PhantomData<Rc<()>>,
}
// Copy, no Drop, private construction. Still one machine word.

struct Scope<'s> { /* root slab, invariant scope/isolate brand */ }
struct Read<'a> { /* exclusive execution capability borrowed for 'a */ }
struct RVec<'a, T> { /* checked object, Read lifetime, no unconditional Deref */ }

enum View<'a> {
    Null, Logical(RVec<'a, i32>), Integer(RVec<'a, i32>),
    Real(RVec<'a, f64>), Complex(RVec<'a, Rcomplex>),
    Character(StrVec<'a>), List(ListView<'a>),
    Environment(EnvView<'a>), Promise(PromiseView<'a>),
    Closure(ClosureView<'a>), Call(ConsView<'a>), /* remaining SEXPTYPEs */
}
```

The brand proves that a root exists in a live scope, not merely that the isolate exists. Every safe creation of `Value<'s>` registers its pointer in that scope or refers to a registered immutable root slot whose lifetime is at least `'s`. Copies need no extra roots. Reading a child field produces a short `Read`-bound value; retaining it across a mutation or allocation promotes it into a scope root. It is unsound to borrow the lifetime of the parent alone: replacing the parent's edge could otherwise collect the child.

Use a generative closure to create scopes; no safe constructor accepts raw pointers, and values from different isolates cannot be mixed. Scope roots live in linked stable slabs, not a reallocating array whose addresses C or GC retain. `scope.escape(child_value, parent)` roots a result in the parent before dropping the child scope. Allocation installs the result in a reserved root slot before any further safepoint. Ordinary Rust result returns transfer this root, including error payloads.

This is a thin object pointer plus a separate root discipline, not a handle-table object representation. C receives the pointer directly. The runtime's VM can use private raw SEXP slots under one audited frame-root implementation rather than registering a handle for every operand copy.

##### Borrowed reads and callbacks

A `Read<'a>` borrows the execution capability for the whole isolate. It permits non-allocating type/length/attribute reads and ordinary-vector slices. It prohibits allocation, GC, finalizer delivery, mutation, warning delivery, promise forcing, active bindings and calls into R or C. Merely calling a guard “NoGc” is insufficient: C may write `REAL(x)` without collecting.

`RVec::try_slice(&Read)` returns a slice only for storage whose read pointer is available without effects. Otherwise, end the read, call `prepare_read(&mut Exec, rooted_x)` or `get_region`, then acquire a new read. ALTREP `Length`, `Elt`, `Dataptr_or_null` and materialization may invoke package code; their general forms are effectful even if a particular built-in compact class has a pure fast path. Never invoke a package method while another live slice exists.

Builtin kernels should have two phases: gather/validate/materialize with rooted handles, then run a plain slice kernel without callbacks. If an operation must interleave evaluation and reading, it must retain the owner as a root and reacquire the data after every effect. Region access copies a bounded chunk into Rust-owned scratch when that is cheaper than materialization. A vector is not copied merely because it crossed `.Call`.

No `&Header`, `&Vec<T>` or `&mut Interp` spanning a reentrant native call may imply exclusive access to memory the callback can touch. Store heap data behind raw pointers/`UnsafeCell` in the audited module. Runtime execution uses a short-lived capability over an isolate pointer; its methods end field borrows before entry to C. The unsafe reentry implementation must not fabricate a second `&mut` to an already borrowed entire interpreter. Signals only set flags; they never execute R while a read lease exists.

For mutation, `prepare_write` first handles dispatch, checks, duplication and writable ALTREP access. Its subsequent short write lease excludes all other read/write leases. Numeric slices can be mutable within this lease. Object-vector writes remain typed setters using `Sexp::store`; exposing `&mut [Sexp]` would bypass reference counts and barriers. Safe code cannot make two simultaneous mutable views by copying a handle.

##### Copy-on-modify

Liveness and sharing are distinct. Roots and copies of `Value` do not increment REFCNT. Heap edges and bindings follow GNU R's `FIX_REFCNT`, including `TRACKREFS` exceptions and saturating counts (`core/src/lib.rs:605–639`; `R/src/main/memory.c:1282`). Sweeping does not decrement children; do not try to make this an exact ownership count.

Introduce `AssignTarget`/`OwnedArg` capabilities in runtime assignment code. A builtin receives rooted arguments by value, and may consume an argument slot without cloning an ownership edge. However, `Copy Value` is not permission to mutate. `ensure_writable` checks sharing, immutable constants, object type and storage policy. Duplicate with GNU R's shallow/deep rules, preserve attributes appropriately and route `tracemem` through this same duplication seam.

A temporary held only by an execution root can still require semantic sharing protection: for example, `x` retained before forcing an index whose expression reassigns `x`. The evaluator must establish GNU R's temporary REFCNT/immutability protections at the corresponding operations; roots alone do not implement copy-on-modify. Port those operations from `eval.c` and `subassign.c`, and test this case explicitly.

Do not remove a visible environment binding while arbitrary code can run. Validate and force in R's order, then transfer the binding's ownership contribution in a callback-free commit step if the replacement fast path permits it. Slow paths use GNU R's normal assignment protocol. This replaces the current `Rc::make_mut` mechanics, not their spelling.

#### 2. Precise Rust roots, conservative C segments

Choose the hybrid already suggested by the working implementation: precise Rust/VM/JIT roots; bounded conservative scan of active C segments; PROTECT and preserve roots remain exact. `docs/ledger/core.md:2260` records why scanning interpreter frames delayed finalizers. The November 11 entry (`:7`) shows that even trampoline padding and the one-past rule required fixes. Reopening whole-Rust-stack scanning would regress a measured semantic property.

A root frame records stable storage, initialized slots and live pointer slots. AST functions and builtins use scopes; VM frames register their operand/binding arrays once and publish liveness before each safepoint. Release dead slots at their last observable use, not only at function return. Otherwise a precise scanner can still delay a finalizer through stale but registered roots. Large builtin loops use nested scopes or reusable slots, not one growing root arena until the entire operation ends. Contexts, condition payloads, code constants, cached closures and deferred finalizers are explicit roots or traced heap edges.

No Rust compiler assumption is needed: the collector reads published memory through its registered frame descriptors. A raw pointer left only in an LLVM register or transformed into an integer is not a root. A derived data pointer must have its base SEXP published before a safepoint. C retains the existing register spill and interior-pointer policy; Rust must not depend on it.

The precedent is explicit roots rather than a new collector: V8's `src/handles/handles.h` defines scope-managed indirect handles; Julia's GC correctness documentation distinguishes rooted values and safepoint-free code and points to `src/clangsa`. Borrow their separation of concerns, not a promise that their whole implementation matches robigo. [V8 source](https://raw.githubusercontent.com/v8/v8/main/src/handles/handles.h), [Julia GC correctness](https://docs.julialang.org/en/v1/devdocs/gc-sa/). GNU R's `PROTECT` stack is the local operational precedent (`docs/core.md:241`).

Every R→C transition publishes precise argument/result slots and opens a C segment. A callback closes that segment after register spilling, enters Rust with rooted arguments, and restores it on return. A deeper native call opens another segment. Use the existing trampolines (`core/src/unwind.rs:1`) rather than designing a second unwind mechanism.

Require compile-fail tests for escaping a scope, crossing a mutator operation with a slice, mixing isolates and storing a borrowed child across parent mutation. Require optimized-code torture tests: collect at every allocating helper and callback, test gcc/clang C shims, and poison freed memory. A debug no-callback/no-GC counter catches capability violations in unsafe paths. Miri tests pure access/root logic; it is not evidence about the C stack scanner.

#### 3. Environments: efficient slots with canonical exposure

Keep ENVSXP identity stable, ENCLOS at +24, header/attributes unchanged. Reserve +40 for a traced frame descriptor. Keep +16/+32 nil while the frame is private; after exposure they are the actual FRAME/HASHTAB objects. The existing environment shim is ready for this distinction (`core/src/lib.rs:778`; `docs/ledger/core.md:506`).

Proposed `core/src/frame.rs` owns storage and tracing; `runtime/src/env.rs` owns R lookup/evaluation semantics. Prototype two layouts before fixing the allocation choice:

* Baseline: ENVSXP plus a compact internal frame allocation reached through +40.
* Candidate: an enlarged environment allocation with private tail slots, a tracing flag and fixed ENCLOS. This saves one allocation on common calls, but needs a collector path that traces its tail. Keep it only if general-call measurements win.

Concrete baseline frame layout, proposal rather than existing ABI:

```text
internal frame: +0 header(16), +16 shape_id:u64, +24 shape_version:u64,
 +32 value_version:u64, +40 function_version:u64,
 +48 len:u32, +52 capacity:u32, +56 mode/flags:u64,
 +64 entries: {symbol:Sexp, value_or_cell:Sexp, flags:u32, reserved:u32} [24 bytes]
```

Use an internal allocation kind with an explicit trace descriptor, not an ordinary RAWSXP containing invisible pointers. Its kind never reaches public TYPEOF, serialization or object.size; aux is the only external route to it and no C accessor exposes aux. The collector traces entries and attributes according to that descriptor; small/large allocation accounting still uses actual bytes. This is a deliberate extension to core and a prototype decision. An implementation using only existing VECSXP metadata is a valid comparison control, but must report its extra allocations.

Start with boxed slot values. Add unboxed binding payloads only after the same expansion protocol used for promises is established. Private slots are searched linearly when small; large frames get an open-addressed symbol-to-slot index with tombstones. Symbols are canonical SYMSXP pointers; a dense internal symbol ID can remain as an index optimization, without another symbol identity system. Preserve GNU R's visible enumeration and hash ordering separately from the fast index; today's `env/table.rs` and `env.rs:63` already demonstrate why that order is not an implementation detail.

##### Exposure is a one-way representation change

`FRAME`, `HASHTAB`, `R_findVarLocInFrame`, binding-cell access and any equivalent inspection can expose mutable cells. Materialize canonical cells once, transactionally while the environment and new cells are rooted. Convert slot entries into references to those cells, removing their former direct value edges. Thereafter cell CAR is authoritative, and slot loads read it. Never maintain a copied cell list that needs write-back. Preserve the original unhashed/hashed representation and enumeration order.

This matters even though no census package imports FRAME: GNU R exposes binding locations (`R/src/main/envir.c:878–947`). A C caller can retain a location and mutate it later. Stable locations cannot be a pointer into a growing Rust vector. Deleting a binding detaches its cell as GNU R does; reinsertion does not silently repurpose an exposed cell. Direct FRAME spine changes force rebuilding or abandoning the derived index.

Private flags represent missing/default, active, locked and deleted state. Exposed cells use GNU R's gp bits. Active binding reads/writes call the evaluator and invalidate read leases. Environment locks prevent structural mutation; binding locks constrain assignment. `<<-` starts in the enclosing environment, searches in R's order, respects active/locked bindings and falls back to global according to `setVar`; it does not blindly store through a cached slot.

Use separate structural, value and function-binding versions plus an isolate search-path epoch. Guard entries record the relevant versions and a stable object identity, with explicit weak or strong tracing. Function lookup caches must preserve `findFun` skipping non-functions and promise forcing. Global cache entries mirror `R_GlobalCache`'s binding-location/base-symbol/unbound cases (`R/src/main/envir.c:616–772`) and invalidate on attach/detach, definitions, removal and enclosure changes.

Initially make exposed frames ineligible for cached value/function assumptions across any effectful call. Load their canonical cells afresh. This is sound even when a C pointer store bypasses a setter. Later, measured setter notifications can narrow invalidation; they cannot justify caching across undetectable raw writes. Conservatively bump native-write and search epochs at C entry, callbacks and return. Binding-cell changes within a callback must be visible before the next callback.

#### 4. Closures, promises, argument matching and contexts

CLOSXP stays 48 bytes: +16 formals, +24 body, +32 environment, +40 compilation metadata. Formals and calls remain real cons cells. A compiled call may retain a flattened argument plan keyed by canonical formals/body identity and mutation version; it never replaces the visible cons graph. `SET_BODY`, `SET_FORMALS`, `SET_CLOENV` invalidate metadata. Mutations to an exposed body/formal spine also invalidate plans, conservatively across native effects until finer tracking is proven.

Metadata is a traced descriptor holding SEXP constants and an owned code allocation. Do not hide pointer-bearing `Code` fields in an opaque Rust `Box` without a trace function (`runtime/src/compile.rs:782`). Native-code release occurs after sweep at a safe point. An executing frame roots its code descriptor, preventing code from being freed mid-call.

PROMSXP remains +16 PRVALUE, +24 PRCODE, +32 PRENV, +40 metadata. Start the prototype with canonical boxed PRVALUE. Preserve the current unboxed fast path as a second, measured variant (`value.rs:636`; `interp.rs:708`): reserve an `extra` tag for private forced-int/real/logical payload at +16. The collector must skip +16 in that state; current `gc.rs:1349` traces it unconditionally, so changing only accessors would be memory-unsafe. PRCODE, PRENV and aux remain traced.

All public PRVALUE reads expand a scalar once into a rooted ordinary vector, publish it in +16 and clear the scalar tag in a callback-free step. Subsequent readers receive that same object. SET_PRVALUE discards the scalar state correctly. Update all robigo headers and inline accessors; if any supported observer cannot be routed through expansion, keep that promise boxed. Never treat scalar bits as a SEXP or copy a whole node using pointer-only assumptions. Serialization, duplication, debugging and GC must understand the state. A forced promise clears PRENV according to GNU R, while retaining PRCODE for substitute.

The important optimization is retaining unboxed values in the VM, not requiring every visible promise to have an exotic representation. Keep caller-forced strict-prefix promises only under today's effect/guard conditions; do not force an argument earlier if a warning, error, dispatch, missingness or reflection can expose the change (`docs/ledger/perf-vm.md:2048`). A default belongs to the callee environment, an ordinary argument to the caller; `...` is a canonical DOTSXP chain preserving tags and missing arguments. No “constant default” optimization may later manufacture an unforced promise after R already read it; `docs/core.md` records that current gap.

Port forcing from `R/src/main/eval.c:954`: recursion detection, PRSEEN state, interrupted-promise warning, pending-promise stack and successful memoization. Preserve state even when that warning raises. C-built and runtime-built promises are the same objects; delete `from_c` and synchronization hooks. Argument matching keeps exact, partial and positional passes, dots rules and verbatim diagnostic calls; compile reusable match plans only under guards and use `interp.rs:1237` as the existing semantic implementation to transfer.

One logical context chain serves all tiers and C introspection. Move current `Interp::Context` (`interp.rs:51`) and capi save/restore (`errors.rs:440`) behind `runtime/src/context.rs` with a core-owned C-compatible prefix or projection defined in robigo's `Defn.h`. Addresses of exposed records stay stable: linked slabs or pinned records, not pointers into a reallocating Vec. The C layout is generated from named fields and checked with C/Rust size/offset assertions; there is no reason to freeze hand-written offsets for this new record.

Each record carries kind/target, call, function, closure environment, sysparent, promised arguments, on.exit actions, srcref, handlers/restarts, visibility, evaluation depth, pending-promise top and saved PPStack/vmax/interrupt state. Generated frames can defer materializing visible records only when escape/reflection guards prove this unobservable. `sys.function`, `sys.call`, `parent.frame`, restarts, browser and on.exit read this same logical chain.

Keep `Result<T, Flow>` in Rust. Flow payloads are rooted while crossing callbacks or executing on.exit. Native entry uses existing C trampolines. Example: Rust A calls C B; B calls `Rf_eval` into Rust C; C calls C D. D's error jumps to D's nearest trampoline, returns Flow through C's Rust frames, then the `Rf_eval` export throws after those Rust frames finish, reaching B's trampoline. No longjmp crosses a live Rust scope guard. Restore pending promises as `R/src/main/context.c:170` does, including PRSEEN=2. Preserve `R_UnwindProtect` continuation ownership and pop-before-cleanup behavior. Keep and extend `tests/no_inner_jumps.rs` when the bridge disappears.

#### 5. Strings

Replace RString/RStr with a checked CHARSXP handle; NA is the immortal NA_STRING pointer, not `None`. STRSXP payload is an array of canonical pointers. Read-only string views expose bytes, encoding and NA state. `as_utf8` is validated and fallible; no universal `Deref<str>` decodes bytes silently.

Use `core/src/chars.rs` rules for ASCII/native/UTF-8/Latin-1/bytes marks and weak interning. Translation belongs at the GNU R operation's translation point, using the existing sysutils ports, not on every string read. Move shared translation helpers to `robigo-text` or a lower module so runtime and capi call one implementation; leave C scratch-buffer lifetimes in capi. CHARSXP +8 hash is private cached hash; TRUELENGTH at +24 remains package scratch (`docs/core.md:356`).

`paste` computes target encoding and sizes, allocates output scratch, then copies bytes and interns final results once. Region/iteration roots protect each CHARSXP across output allocation. `match`/`unique` use pointer equality as the same-encoding fast path, then GNU R's encoding-aware equality/hash normalization: distinct encodings can compare equal. Hash consistency must cover Latin-1 versus UTF-8 equivalents, native locale behavior, bytes and NA. Cache normalized keys only with a locale epoch and correct roots; avoid allocating a normalized Rust String on every probe.

Measure repeated strings, high-cardinality strings, long strings, mixed encoding, invalid byte input and NA-heavy data. The shorter handle does not by itself prove a faster `paste`; interning and translation could dominate.

#### 6. Vectors, ALTREP, attributes and growth

Keep the current header and offsets: type +0, flags +1, gp +2, REFCNT u16 +4, extra +6, attributes +8; ordinary vector length +16, truelength +24, payload +32; node aux +40 (`core/src/lib.rs:165–180`). These are the code's offsets, not architecture.md's older u32 refcount sketch.

Ordinary vectors use inline data. Compact sequences remain built-in ALTREP throughout AST, VM and C; only writable or mandatory contiguous access materializes. `1:2e8`, slicing a few elements, and crossing a native element-access call must not allocate the sequence. Foreign buffers use ALTREP, not EXTDATA: writable REAL/INTEGER access must detach a read-only/shared foreign buffer once (`docs/core.md:177`). Bridge-owned EXTDATA disappears; an independently justified writable external owner can remain as a core storage facility.

ALTREP dispatch lives below runtime/capi's common safe interface, with fallible Rust methods for built-in classes and trampoline calls for package methods. Length and region operations can fail. Mutations invalidate cached views; no buffer pointer survives reentry without reacquisition. Keep GC non-moving and never realloc a published inline payload behind C's raw pointer.

For append replacement, use GNU R's growable/truelength policy from subassign.c, not an arbitrary doubling constant. If unshared and spare capacity permits, update length in place. On overflow, allocate a larger object, copy, initialize new cells, and rebind; old object/data stay alive as long as roots or C ownership require. Repeated `x[n+1] <- v` is amortized through geometric spare capacity, while shared vectors duplicate per R semantics. An unpublished construction builder may use Rust storage before sealing, but cannot be passed to C as a moving R vector. Trace only initialized logical elements; clearing shortened reference-vector tails and their count accounting must follow the chosen R operation.

Attributes are one canonical pairlist. `attrib.rs` continues R validation, OBJECT/S4 synchronization and copy-most rules. Any name/dimension lookup cache is derived and invalidated by attribute/spine mutation. Keeping old `Attrs(Vec<...>)` as a second writable store would recreate split identity.

#### 7. VM and JIT

Retain `RVal` as an execution-only sum type (`vm.rs:41`): boxed SEXP, int, real, logical and compact-range descriptor. Use explicit tags; do not NaN-box because R observes NA/NaN payloads. Prototype a 16-byte `{payload:u64, tag:u8, padding}` register plus side descriptors for ranges, against split tags/payloads. Initialize tags for all root-visible registers before allocation. A tag distinguishes pointers for tracing; no scan of numeric payloads.

Register files live in reusable linked frame segments with live maps. The scanner sees only initialized live boxed slots and published code/environment/context roots. Do not carry forward SmallVec/drop/reserve code merely because it was previously tuned. Preserve the measured semantics listed at `docs/ledger/perf-vm.md:2048`: local function lookup, effect-free closure inlining, caller-forced promises, index forcing order, search/environment/function guards, rejection memos, unboxed returns and the exact row-vector matrix kernel. Preserve whole-closure/native call trees (`jit/closure.rs`), nested pure-call inlining and simple-internal plans. Keep hot dispatch small; redo cold-path placement against new codegen.

Proposed generated ABI:

```rust
unsafe extern "C" fn code(
    exec: *mut ExecState,
    frame: *mut JitFrame,
    args: *const Reg,
    result: *mut Reg,
) -> ExitCode;
```

`JitFrame` publishes previous frame, code owner, safepoint/deopt index, live boxed-root slots and execution slots. Before any allocating/calling/error helper, spill every live SEXP into its published root slot. Helpers can collect immediately. After a helper, reload pointers and recheck guards. Nonallocating arithmetic regions keep registers unboxed without root traffic. Insert interrupt/safepoint polls in long pure loops; publish roots before them too.

Generate header constants from core and assert them against C headers; never duplicate numeric offsets in Cranelift builders. Guards cover SEXPTYPE, flags, attributes, scalar shape, environment versions, numeric mode and storage class. An ALTREP pointer or a writable pointer from a previous native effect is not an ordinary inline vector.

Deopt records map every live value, promise state, environment slot, visibility and PC to VM state. Box scalars while their frame is still registered; then transfer roots and retire the JIT frame. Resume at the operation that has not committed. Replay a whole native call tree only when the existing purity rules prove nothing observable occurred; never replay a warning, mutation or callback. Rebuild current `VecView`/`LoopCtx` buffer machinery (`jit.rs:121`) around rooted owners and short pointer validity, rather than keeping old buffers alive as a substitute for an aliasing contract.

#### 8. Isolate and crate ownership

Move all mutable runtime state into one owning isolate: Core, globals/protect/preserve state, symbol/CHARSXP tables, evaluator contexts and conditions, environment epochs/cache, VM/JIT caches and frame pools, connections/DLL registrations, RNG, locale, options, source references, formatting scratch, visibility and interrupts. Inventory the 69 thread_locals from the brief; classify each as immutable process state, isolate state or entry-only TLS. Do not move them as a bag of RefCells without identifying reentry lifetimes.

There is one TLS active-isolate pointer at external entry, saved/restored for nested entry. Internal Rust and generated code pass an execution pointer/capability directly; arithmetic, lookup, store and allocation must not repeatedly fetch TLS. Existing capi Tables already group state and document that no borrow crosses package/R execution (`capi/src/state.rs:15`). Generalize that rule.

Put memory, raw objects, roots, frame storage, borrow capabilities and low-level services in core. Runtime owns evaluation semantics; capi owns exports and trampolines. Lower shared fallible vector/string/environment storage operations out of capi where runtime needs them. Capi can continue calling runtime for evaluation, with installation/wiring in CLI; no circular dependency is necessary. The existing bridge contains service wiring as well as conversion, so delete conversion and relocate required callbacks rather than deleting exports wholesale.

C mutable data symbols become macros/accessors over per-isolate state as source compatibility permits. Audit direct base-package macros too. Do not share today's mutable symbols or attribute-bearing “immortal” constants as read-only process data without redesign: `docs/core.md:310` says their children are mutable. Initially isolate-own such objects; share only proven immutable bytes/code/constants. Sharing symbols later requires isolate-local value cells and synchronization, as core.md §10 anticipates. C packages with process-global mutable state continue using process workers unless audited for isolates.

Revisit a real libR.so only after TLS consolidation, with the November 10 executable and shared-library experiments repeated. Shared linkage is not a G3 semantic prerequisite. The measured median regression is documented at `docs/ledger/core.md:160`; do not trade it away for packaging neatness.

#### 9. Migration and stopping rules

##### Main before conversion

1. Land the interface specification, root/borrow compile-fail tests, header/trace assertions and identity/unwind regressions. Establish reproducible baseline binaries and package fixtures at the supplied commit.
2. Land isolate ownership changes in independent behavior-preserving steps. Keep their benchmark and ratchet checks separate from representation changes.
3. Build the prototype as a separate crate/executable using core, with no Rc runtime imports. Keep it on main behind an explicit target; ordinary users still run the current runtime.
4. Apply narrowly useful prep to the current runtime: separate pure slice kernels, remove public payload mutation, centralize assignment/duplication, inventory effectful operations. Do not require a 4,900-site generic `view()` rewrite before measuring the new interface. A read-only enum view that still exposes Rc internals is not the final seam.

##### Integration branch

Use one default representation on the branch, not module-by-module calls through a mixed heap. First convert Value/Sym/Env/Arg, rooting, allocation, attributes, cons cells, basic evaluation and conditions. Then bootstrap/coercion/printing, builtin families, native entry, VM and JIT. Keep existing semantic algorithms where correct. The separate prototype becomes the replacement runtime substrate, not a permanent second implementation.

Use a syntax-aware codemod for constructor names and mechanically recognizable match patterns. Compiler errors enumerate remaining sites. Convert each builtin module with an effect/rooting audit: reads surviving allocation, index/argument forcing order, setter ownership, ALTREP assumptions, attribute snapshots and callbacks. `Rc::make_mut`, direct `.data`, `RefCell` borrows and escaping references require human reasoning; regex replacement is not approval. Keep a per-module checklist and tests in the branch, avoiding another generic adapter layer merely to make code compile.

Main continues oracle/test additions, independent numerics/syntax work, build/CI fixes and urgent correctness fixes. Pause new builtin ports and Rc-layout optimizations now, consistent with `docs/design.md:587`; also pause VM register-storage redesigns that the conversion replaces. Bring semantic fixes and tests into the branch regularly, with one integration owner resolving them. Feature work resumes as modules stabilize.

Branch lifetime is milestone-bound, not an invented calendar promise: open it only after the prototype passes; end it after one coordinated conversion campaign. Weekly checkpoints report unconverted modules, ratchet failures, root violations and performance deltas against the frozen baseline plus current main. If two checkpoints fail to reduce either converted surface or failures, stop expanding scope and repair the failing seam. Do not sustain two runtimes as parallel products for months.

Main's ratchets stay green throughout. An incomplete integration branch cannot truthfully pass the full ratchet; report failures, never remove markers or mask them with old-runtime fallbacks. Converted slices have a growing required green set; the final branch runs every existing ratchet in every tier, base-native mode, package-install and package-test gate before default cutover. G3's documented full-conformance goal also requires resolving remaining failures or explicitly distinguishing incomplete G3 from a representation-cutover milestone; preserving 600/622 alone does not mean full conformance.

Delete bridge proxy/twin/hold tables, dirty writeback, EXTDATA Rc owners, runtime cycle inventory, retirement lists, promise synchronization and conversion caches before merging the default cutover. Retain and rename the native callback/export wiring. Prove deletion with symbol/source inventory and counters showing zero conversions and one identity. Run the named identity bugs as regressions, including nested callbacks, not merely their top-level reproductions.

##### Mixed heaps and alternatives

The coordinator's rejection is correct as a production direction, but “unsolved” is too absolute. `docs/core.md:712` already describes one mark over both graphs for explicit gc, and the ledger documents limited cycle reclamation. A fully traced cross-heap design is possible; it would require exact tracing of every Rc owner, retention reconciliation and identity rules for every mutation. That is building another memory manager and extending the bridge's hardest mechanisms. Attributes are one route among many, not a proof of impossibility.

Two separate executables/crates during development are not a mixed object graph. This is the preferred staging alternative: compare them through R inputs and recorded outputs, never pass values between them. A permanent runtime-v2 with wholesale reimplementation of all builtin semantics is unnecessary; transplant established semantic code into the proven new substrate.

#### 10. Prototype and measurement gates

Build a vertical slice, not a toy arithmetic interpreter. It needs real core objects, scopes, environment lookup, closures, three-pass matching, ordinary/default promises, `...`, scalar arithmetic, replacement, contexts/errors and a minimal C entry/callback. First run fib/binarytrees and simple call shapes; then add exactly the semantics needed to run all of `benchmarks/suite/calls.R`, including S3/tryCatch cases. Label unsupported cases; do not substitute simplified scripts and claim full coverage.

Compare both AST-to-AST and VM-to-VM before interpreting overall numbers. A new AST losing to today's pure-call JIT says nothing useful about GC. Port a minimal execution fast path to measure unboxed calls, then bring over existing optimizations for the final speed comparison.

Variants settle these choices independently:

| Decision | Variants | Evidence |
|---|---|---|
| Root cost | scope roots; pre-sized frame roots; whole-stack scan as diagnostic control only | Ir per call, root stores, live/dead retention, finalizer order |
| Frame allocation | separate frame; environment tail; canonical cells everywhere | calls.R, active bindings, escaped frames, lookup misses, bytes/call |
| Promises | boxed; private scalar with canonical expansion | defaults, strict args, substitute/missing, C PRVALUE mutation |
| Registers | tagged slots; split tag/payload | dispatch Ir, cache misses, root scan time, deopt |
| Payload reads | slice phase; region kernel; eager materialization control | large vectors, compact ranges, package ALTREP callback counts |
| Isolate access | direct execution pointer; current TLS baseline | empty/general calls, native exports, startup |

Use current scripts for the existing runtimes, for example `scripts/perf.py run --metric instr --filter 'calls|fib|binarytrees'`; inspect `scripts/perf.py --help` for artifact/runtime path options before executing. New prototype instrumentation should report instructions per operation using N versus 2N runs, allocations/bytes by object kind, root stores/scans, GC count, live/reserved heap, peak RSS, pause distribution and maximum, and materializations. Use callgrind cache simulation as the ledger did; estimated cache penalties supplement, not replace, hardware wall measurements. Interleave release runs on the same machine, record compiler/build/oracle configuration, preserve stdout/stderr/status, and keep startup separate. No quick-profile speed results.

Add `paste`, `match`, `unique`, list growth, cyclic closures, repeated `.Call` no-ops and nested callbacks. Include a package subset that stresses identity (data.table), promises/environments (rlang), loop-index mutation (purrr), conditions/unwind (Rcpp/cpp11) and pointer lifetimes. Run all 42 source installs and the full package suite for final acceptance, using the coordinator's figures as baseline rather than claiming they were re-measured here.

Proposed entry gate for the full conversion:

* All slice semantics match GNU R; scope/borrow misuse fails to compile; torture and nested C→R→C unwind pass; finalizers are not retained by dead Rust roots.
* Representative general calls are no slower than the current runtime at equal execution tier, and their aggregate is no slower than GNU R. Require a meaningful improvement in at least the environment/default-promise cases, not just fib.
* No unexplained increase in peak RSS or pause tails. Use a proposed 10% increase as an investigation trigger, not a waiver for unbounded memory. Repeated cycles plateau after warmup.
* Rooting and read interfaces can express representative allocation-heavy and callback-heavy builtins without broad unsafe blocks or persistent slice copies.

If that gate fails, improve the prototype before mass conversion. The final merge keeps the repository's existing instruction gate: no benchmark more than 0.5% slower, measured against cumulative baseline as well as recent main (`docs/ledger/perf-vm.md:3`). Require package timing no worse in aggregate and explain individual changes. G3's additional gates remain constant-memory compact sequences, byte-string round trips and bounded leaks. The 2.95x suite speedup must survive; a faster toy evaluator cannot waive this.

#### 11. Ranked risks and unresolved decisions

1. **Unsound read/root/reentry interface.** Highest impact: use-after-free or Rust aliasing UB. Resolve with a real prototype implementation, compile-fail tests, Miri-compatible core tests and optimized torture across nested callbacks. Do this before codemodding builtins.
2. **Environment exposure recreates split state.** Resolve with retained varlocs, FRAME/CDR/SETCAR mutation, active bindings, removal/reinsertion and C mutations between two Rust callbacks. Compare private slots, canonical cells and one-way exposure on calls.R. Keep canonical cells if the faster shape cannot preserve semantics.
3. **GC roots delay weak-key/finalizer behavior.** Resolve with last-use root clearing, existing weak-keys tests and varying call depths/tiers. Precise roots are necessary but not sufficient. Preserve the C-segment tests from November 11.
4. **General-call performance loses to metadata and root traffic.** Resolve with per-call instruction and allocation attribution. Frame tail allocation and reusable root slots are real alternatives; the presumed speedup from thin Value is not evidence.
5. **Unboxed promise/binding expansion misses an observer.** Resolve by auditing headers, duplication, serialization, inspector, GC and PRVALUE setters. Prototype boxed promises first. Drop the optimization if observer coverage or measured benefit is inadequate; keep VM scalars.
6. **C raw writes defeat speculative guards.** Resolve with exposed-frame slow paths and effect epochs, and tests that hold data/cell pointers across native calls. Generational GC remains deferred until raw pointer-bearing vectors are tracked as always dirty. Existing barrier hooks alone are not sufficient.
7. **Metadata caches leak or have stale identity after reuse.** Resolve with traced/weak ownership, stable IDs or validated live owners, code eviction stress and heap-address reuse tests. No address-only global maps without lifetime handling.
8. **Migration mistakes hide in broad match rewrites.** Resolve with module-level effect inventories and preserved ratchets in all tiers. Native code, reflection and replacement builtins need manual review even when a codemod compiles.
9. **Isolate work changes packaging without eliminating TLS cost.** Resolve with instruction/disassembly checks for TLS reads, nested-isolate tests and the shared-library experiment after consolidation. Separate untrusted package globals by processes.
10. **Collector/RSS policy no longer matches the bake-off.** Resolve with real core layout and actual GNU R triggers (`docs/core.md` §3), cache-sim results and allocation/GC latency profiles. Revisit allocator/frame layout first; do not turn this into a collector replacement without evidence.

The design's firm choices are canonical heap identity, precise Rust liveness, callback-free borrowed reads and a shared semantic implementation for C and Rust operations. Frame packing, scalar promise storage and register packing remain measured choices. That distinction lets G3 remove the bridge's failure modes without treating an untested layout as a requirement.


### Fable

### G3 design: robigo's runtime on robigo-core (Claude Fable 5.2)

Base: main `4cff0ceb`. Read-only review. Counts are from `rg` over
`crates/robigo-runtime/src` unless a file is named. Guesses are marked
"(guess)". GNU R paths are under `~/.cache/robigo-research/rsrc/R-4.5.0/src`.

#### 0. Verdict

Ten decisions. The first four differ from the coordinator's plan or from
`docs/core.md`.

1. **Rooting: precise roots decide what is reachable; the conservative scan
   only pins memory.** A collection marks from the precise roots, decides weak
   references and finalizers, and only then marks from the conservative words.
   An object reached only by a conservative word stays allocated and is treated
   as dead for finalizers. Stale stack words then cannot delay a finalizer, in
   Rust frames or in C frames, and a missing root in Rust code cannot free
   memory under a live pointer. CI runs with the Rust-frame pins off and
   gctorture on, so the precise discipline is still checked (§2).
2. **No long-lived integration branch.** The G3 handle API lands on main first,
   implemented over today's `Rc` model. The builtins (about 85k of the 115k
   lines) are converted to it on main, module by module, under today's
   ratchets. The new evaluator, environments, VM and JIT grow on main behind a
   cargo feature `g3`, with a ratchet of their own. When the `g3` ratchets
   match main's, the default flips and the old backend and the bridge are
   deleted (§9). The coordinator is right to reject a mixed heap. The claim
   that the swap can be "local to `value.rs`" is wrong: ownership (`Rc` counts
   every holder, `REFCNT` counts heap edges only), strings, environments and
   the evaluator's in-place protocol do not hide behind `Value`.
3. **GNU R's `REFCNT` protocol, ported exactly, with GNU R's deferred link
   counts for the evaluation stack** (eval.c:995-1056). `Value` becomes
   `Copy`. `Rc::make_mut` becomes `if MAYBE_SHARED(x) x = shallow_duplicate(x)`
   at the places the C source has it. AGENTS.md's porting rule 5 is rewritten.
4. **Variables live in the environment object, in inline typed slots**, not in
   VM registers with a flush. One allocation per closure call (48 + 16n
   bytes). Bindings hold unboxed scalars as GNU R's typed binding cells do
   (Defn.h:462). This deletes `Vars`/`flush_pending` (vm.rs:301-366) and makes
   `get()`, `sys.function` and C's `findVar` see current values with no
   write-back.
5. `RResult<Value>` is one machine word: `Value` is `NonNull`, the error type
   is a zero-sized `Unwind`, and the pending exit lives in the isolate, where
   the collector sees it.
6. Hashed and user-made environments keep GNU R's representation bit for bit
   (FRAME pairlist, HASHTAB, binding cells). `ls(sorted = FALSE)` order is
   then GNU R's by construction and `env/table.rs` (360 lines that simulate
   GNU R's hash order) is deleted.
7. Strings are CHARSXPs. `match`/`unique` hash pointers as unique.c:207 does.
   The symbol of a string is cached in the CHARSXP's spare word.
8. Native code needs no stack maps: it keeps objects only in value-stack and
   environment slots, and the heap does not move.
9. `Interp` becomes the isolate record. Hot paths get it as `&mut Interp`;
   only C entry points read the one thread-local.
10. The prototype measures six decisions and has numeric exit criteria (§10).
    One of its experiments, the two-pass mark, can run on main today.

#### 1. The Rust-side object API (council item 24)

##### 1.1 `Value`

```rust
// crates/robigo-runtime/src/value/gc.rs   (feature "g3")
###[repr(transparent)]
###[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(NonNull<u8>);          // the address C holds as SEXP

impl Value {
    pub fn sexp(self) -> robigo_core::Sexp;
    pub fn typ(self) -> u8;                         // one byte load
    pub fn view(self) -> View;                      // for matching
    pub fn same_object(self, o: Value) -> bool;     // pointer compare
}
```

- A plain `Copy` pointer. No lifetime brand, no handle table.
- Lifetime-branded handles (V8's `Local<'s, T>`, `src/handles/handles.h`;
  SpiderMonkey's `Rooted`/`Handle`, `js/public/RootingAPI.h`; file names from
  memory) exist because those collectors move objects. robigo's heap does not
  (core.md §3). A brand would put a lifetime on every `Value` in 115k lines
  and in every struct field, to prevent a failure that §2 handles another way.
- `robigo_core::Sexp` stays a nullable raw pointer for core and capi (C can
  pass NULL). `Value` is the runtime's non-null form, so `Option<Value>` and
  `Result<Value, Unwind>` are one word (Rust documents this layout for a
  `Result` whose error is a field-less 1-ZST).
- `NULL` is a static object (§1.6), so `v.is_null()` compares with a link-time
  address.

##### 1.2 Matching: `view()`

```rust
pub enum View {
    Null, Sym(Sym),
    Lgl(Lgl), Int(Int), Real(Real), Cplx(Cplx), Str(Str), Raw(Raw),
    List(List), Expr(List),
    Pairlist(Cons), Lang(Cons), Dots(Cons),
    Closure(Closure), Builtin(Prim), Env(Env), Promise(Promise),
    ExtPtr(ExtPtr), WeakRef(WeakRef), Obj(S4), Bytecode(Code), Char(RChar),
}
```

Each payload is a `#[repr(transparent)]` `Copy` newtype over `Value`.
`view()` loads the type byte and builds the variant, so
`match v.view() { View::Real(r) => .. }` compiles to the jump today's
`match v { Value::Real(r) => .. }` makes, plus one load from the header (which
the arm needs anyway for the length or the data). The 4,272 `Value::<Variant>`
sites in the runtime (86 files; the brief's 4,900 includes capi) become
`View::<Variant>` on a scrutinee with `.view()`.

`Builtin(&'static Builtin)` becomes `Prim`, a BUILTINSXP/SPECIALSXP object
whose car is the index into the builtin table, as GNU R's `PRIMOFFSET`.

##### 1.3 Typed vectors and borrowed reads

```rust
impl Real {
    pub fn len(self) -> usize;                       // ALTREP-aware (Length)
    pub fn std(&self) -> Option<&[f64]>;             // standard layout only
    pub fn slice(&self, cx: &mut Interp) -> RResult<&[f64]>;   // DATAPTR_RO
    pub fn slice_or_null(&self) -> Option<&[f64]>;   // DATAPTR_OR_NULL
    pub fn elt(self, cx: &mut Interp, i: usize) -> RResult<f64>;
    pub fn regions(self, cx: &mut Interp, f: impl FnMut(usize, &[f64])) -> RResult<()>;
    pub fn attrib(self) -> Value;                    // pairlist or NULL
}
```

`Int`, `Lgl`, `Cplx`, `Raw` are the same. `Str::slice` gives `&[RChar]`,
`List::slice` gives `&[Value]`; both are the STRSXP/VECSXP payload read in
place.

- `std()` is the hot path: one test of the flags byte
  (`ALTREP | EXTDATA | WRITING == 0`), then `x + 32` and the length at `+16`.
  It is the shape of GNU R's `DATAPTR` test (core.md §2).
- `slice()` is `DATAPTR_RO`. It is fallible because an ALTREP class's
  `Dataptr` can raise. It runs the method with collection inhibited, as
  altrep.c:352-366 does, and with a "no evaluation" flag: `Rf_eval` called
  under it raises `robigo: evaluation inside an ALTREP data method is not
  implemented yet`. (Guess: no package evaluates R code in `Dataptr`. The
  package suites settle it.)
- `slice()` on a compact sequence expands it, as `INTEGER(1:n)` does in GNU R.
  Code that must stay in constant memory uses `elt`, `regions`
  (`ITERATE_BY_REGION` of `R_ext/Itermacros.h`) or the iterator types in
  `value/iter.rs`, which have a direct arm for robigo's own compact classes.
  The loops that matter (`for`, `[`, `[[`, `sum`, `length`, `seq_along`,
  `%in%`) use those. This closes #460 and gives G3's `1:2e8` gate.
- The slice's lifetime is tied to the typed handle (`&'a self`), as today's
  `match v { Value::Real(r) => &r.data }` ties it to the `Rc`. So today's
  code keeps its shape.

**What keeps a slice sound.** Four hazards, each with its rule.

| hazard | rule |
|---|---|
| the object is collected | It is rooted (§2), or in release builds pinned by the slice pointer itself. The scan resolves interior pointers and one-past-the-end (core.md §3, item 19). |
| the collector writes the payload | It never does. Marks are in side bitmaps (core.md §10); counts are in the header. A slice may be held across allocation. |
| R code modifies the object in place while the slice is held | It cannot. An object on the value stack gets a link count before any in-place modification (§1.5), so R code sees it as shared and copies. GNU R's C code relies on the same protocol when it holds `REAL(x)` across `eval`. |
| the builtin itself writes it through another handle (`x[i] <- x`) | Writable access goes through `Unique` (below), which sets the header's `WRITING` bit. `std()` and `slice()` refuse an object with the bit set, so a shared slice and a mutable slice of one object cannot exist together. GNU R handles this case with explicit `x == y` checks. |

One hazard is accepted as GNU R accepts it: C that writes through `REAL(x)`
without checking `MAYBE_SHARED` (data.table's `set`) while a Rust caller holds
a slice. The read is stale and in bounds. A slice is not held across a
`.Call` except where GNU R's C does the same.

This API is safe Rust only given the runtime's invariants (rooting, the link
protocol). `value/gc.rs` therefore joins robigo-core and robigo-capi as code
whose rules are core.md's, not the borrow checker's
(`docs/architecture.md`, "Unsafe code"). The strict CI mode of §2 is what
enforces it.

##### 1.4 Mutation and copy-on-modify

`REFCNT` is GNU R's: heap edges and bindings count, locals and the stacks do
not (core.md §3; `Sexp::store`, robigo-core `lib.rs:605`). The runtime uses
GNU R's predicates under GNU R's names: `MAYBE_SHARED` (count > 1),
`MAYBE_REFERENCED` (count > 0), `NO_REFERENCES`.

```rust
pub struct Unique<T>(T);        // not Copy; the only writable handle
impl Interp {
    pub fn alloc_real(&mut self, n: usize) -> Unique<Real>;      // fresh
    pub fn writable<T>(&mut self, x: T) -> Unique<T>;            // MAYBE_SHARED ? shallow_duplicate : x
}
impl Unique<Real> {
    pub fn as_mut_slice(&mut self) -> &mut [f64];
    pub fn set_attrib(&mut self, name: Sym, v: Value) -> RResult<()>;
    pub fn finish(self) -> Value;                                 // clears WRITING
}
impl Unique<List> { pub fn set(&mut self, i: usize, v: Value); } // Sexp::store: barrier + FIX_REFCNT
```

- `writable(x)` is the one spelling of `if (MAYBE_SHARED(x)) x =
  shallow_duplicate(x)`. It reports to `tracemem` when it copies (duplicate.c's
  `memtrace_report`; the RTRACE bit is in the header, which closes #140).
- Today's 142 `Rc::make_mut` sites become `writable`. The difference in
  meaning is deliberate. `Rc` counts a VM register; `REFCNT` does not. The
  evaluator makes up for it with the link protocol (§1.5), not the builtin.
- **A dead container keeps its children's counts** (core.md §3; GNU R does the
  same). Today's `Rc` drops them at once. So G3 will copy in a few places
  where today's robigo does not. That is GNU R's behaviour, and `tracemem` and
  `.Internal(inspect)`'s `REF(n)` show it, so an "improvement" here is a bug.
  `R_CleanupEnvir` and `unpromiseArgs` (eval.c:2132, 2169) are ported exactly:
  they are what lowers the counts of arguments when a call returns.

##### 1.5 Arguments by value, and in-place replacement

```rust
pub type BuiltinFn = fn(&mut Interp, call: Value, args: Args<'_>, env: Env) -> RResult<Value>;
pub struct Args<'a> { /* a run of value-stack slots */ }
impl Args<'_> { fn len(&self) -> usize; fn value(&self, i: usize) -> Value; fn tag(&self, i: usize) -> Option<Sym>; fn set(&mut self, i: usize, v: Value); }
```

- Arguments are slots of the value stack (§2.2), so they are precise roots for
  the whole call and no `Vec<Arg>` is allocated (today: `builtins/mod.rs:63`
  and `Interp::arg_pool`, interp.rs:226).
- A replacement builtin decides as its C source does:

  ```rust
  // R: subassign.c:1633
  if maybe_shared(x) || (!is_assignment_call(call) && maybe_referenced(x)) {
      x = cx.shallow_duplicate(x);  args.set(0, x);
  }
  ```

  `IS_ASSIGNMENT_CALL` is GNU R's bit on the call. `Builtin::in_place` and
  the `InPlace` enum (value.rs:856) go away: they exist because `Rc` needs the
  value moved out of its binding.
- Complex assignment is `applydefine`/`evalseq` ported as written, including
  `DISABLE_REFCNT` on the `*tmp*` cell (eval.c:3442).
- **The link protocol** (GNU R's `R_BCProtTop`/`R_BCProtCommitted`):
  - the isolate has `links_committed: *mut Slot`;
  - every activation (a VM frame or a builtin call) has a base on the value
    stack; the slots below the running activation's base belong to suspended
    code;
  - before an in-place modification of an object reached through a binding,
    the evaluator calls `commit_links()`: if `links_committed < base`, it
    increments the count of every object slot in `[links_committed, base)`
    and moves the mark. One compare when there is nothing to do;
  - when an activation resumes, `uncommit_to(its base)` decrements what was
    committed above that base. One compare on the return path;
  - inside one activation the compiler knows which registers are live across
    an assignment. For a register that holds a variable's value (loaded by
    `GetVar`, not a fresh result) and is live across a later assignment in the
    same frame, it emits `IncLnk r` / `DecLnk r` (GNU R's `INCLNK`/`DECLNK`
    instructions; the AST evaluator's `evalList` does the same at eval.c:3666
    and 3679).
  - Code written in a functional style never pays. A loop that calls a
    mutating closure pays two count updates per live temporary of its own
    frame per call. Frames hold only expression temporaries (variables are in
    the environment, §3), so that is a handful.

##### 1.6 Constants and constructors

- The immortal singletons (`R_NilValue`, `NA_STRING`, `R_BlankString`,
  `TRUE`/`FALSE`/`NA`, `R_UnboundValue`, `R_MissingArg`) and the roughly 300
  predefined symbols (`sym.rs`'s `predefined!`) become **static objects in the
  binary**. `v == NULL` and `tag == s::NAMES` then compare against a link-time
  address with no load. Today each is a field of `Core` (gc.rs:128-133). The
  782 `s::NAME` uses keep their spelling. Core changes: `mark_obj` already
  returns on IMMORTAL before it looks at a block (gc.rs:1256); the store
  barrier's slow path finds the remembered set through the block header
  (gc.rs:229) and needs a branch for static objects.
- Constructors (`Value::real(x)`, `Value::strs(..)`, `Value::list(..)`) keep
  their names and take no context argument. Inside they reach the heap through
  the isolate pointer (§8). Hot paths use `cx.alloc_*`.

#### 2. Rooting Rust frames

##### 2.1 The decision

**Precise roots decide reachability. Conservative words only pin.**

```text
collect():
  1. mark from the precise roots; trace
  2. weak references to a fixed point; decide what is ready to finalize
     (memory.c's CheckFinalizers), on the marks of step 1 only
  3. mark from the pins (conservative scan of the machine stack); trace
  4. prune the CHARSXP cache on the union (a pinned CHARSXP stays cached, so
     pointer equality of strings holds for whoever holds it); sweep
```

Step 3 moving after step 2 is the whole change to `collect_needing`
(gc.rs:1524): a reorder of about twenty lines.

Precise roots: the protect stack, the precious list, the API globals
(core.md §3 roots 1 to 3), and new in G3: the value stack (§2.2), the context
stack (§4.4), the isolate's `Value` fields, and the pending exit (§4.5).

Pins: every aligned word of the machine stack from the stack pointer to the
top, after spilling callee-saved registers, as the scan does today.

Why this and not one of the three options in the brief:

- **Conservative scan of the whole stack as roots** (the bake-off's
  assumption; JavaScriptCore, `heap/ConservativeRoots.cpp`,
  `heap/MachineStackMarker.cpp`; Ruby's `gc_mark_machine_stack`) is ruled out
  by a measurement already in the ledger. Stale words in the evaluator's
  frames kept dead objects alive, so finalizers ran one `gc()` late: R6's
  finalizer tests under rlang's `eval_bare` (core.md §3 root 4; ledger
  2026-10-28 "conservative scan limited to C frames" and 2026-11-11, where a
  `Value`'s seven padding bytes carried the stale pointer). JS and Ruby do not
  promise when a finalizer runs. R's tests do: `rm(x); gc()` and then the
  finalizer's output.
- **Precise handle scopes only** (V8, SpiderMonkey, HotSpot's `Handle`, GNU
  R's `PROTECT`) is what GNU R has, and R needs it for exactness. On its own,
  a missing root is a use after free. SpiderMonkey needed a static hazard
  analysis (`js/src/devtools/rootAnalysis`) to make it hold; GNU R has `rchk`
  and still ships PROTECT bugs. robigo would be converting 115k lines to it at
  once.
- **The hybrid above** takes exactness from the precise set and memory safety
  from the pins. A missing root costs, at worst, a finalizer that runs early
  for an object a Rust local still holds. It cannot corrupt memory.

Consequences:

- Stale stack words can no longer delay a finalizer in any frame, C frames
  included. The remaining gap the ledger names (2026-11-11, "Remaining":
  frames of package code below the cleared 1 KiB) closes.
- The scan segments, the naked-asm stub and the 1 KiB zeroing
  (`Core::close_scan_segment`, `robigo_call65_stub`; +108 instructions per
  `.Call`) are no longer needed for determinism and can be deleted, **if C
  frames become pin-only too**. That changes one behaviour: an external
  pointer held only by an unprotected C local across a collection gets its
  finalizer run. GNU R frees such an object outright, so robigo is still the
  safer of the two. Measure it before deciding (§10, experiment A). If a
  package regresses, C segments stay strong roots as today and only Rust
  frames are pin-only.

##### 2.2 The value stack

One contiguous array per isolate, GNU R's `R_BCNodeStack` generalized
(Defn.h:1609; memory.c:1832-1835 scans it by tag).

```text
Slot (16 bytes):   w0  0..3: scalar tag (0 empty, 1 REAL, 2 INT, 3 LGL)
                       otherwise: an object pointer
                   w1  the scalar's bits when w0 is a tag
```

- It holds VM register files, builtin argument runs, the AST evaluator's
  evaluated arguments, and kept roots (§2.3).
- The collector scans `w0` of every slot below `vstack_top`: a word of 4096 or
  more is an object. Exact and branch-cheap.
- 16 bytes per slot against today's `RVal`, which is 32 bytes by its fields
  (vm.rs:41: `Range` has three 8-byte fields; not measured). `RVal::Range`
  goes: a compact sequence is one 48-byte object in G3, allocated once per
  loop.
- NaN-boxing is rejected: every 64-bit pattern is a double R can hold (NA is a
  NaN with payload 1954 and payloads are compared bit for bit), so pointers
  cannot hide in NaN space without a check on every double store.
- Reserve 256 MiB of address space, committed on demand, with a guard page.
  On wasm32, a fixed array.
- A register is cleared when its value dies (`Op::Null` exists for this
  today, compile.rs:1511), or each safe point has a liveness bitmap. Both give
  the collector only live values. The prototype measures which is cheaper
  (guess: clearing, since most temporaries are consumed by the op that
  follows).

##### 2.3 Roots in builtins: `keep`

```rust
let ans = cx.keep(cx.alloc_list(n).finish());   // pushed on the value stack
for i in 0..n {
    let v = cx.call(fun, &[x.get(i)], env)?;    // may collect; ans is rooted
    List::set_raw(ans, i, v);
}
Ok(ans)
```

- `keep(v)` pushes `v` on the value stack and returns it. There is no guard
  object and no `Drop`: the dispatcher records `vstack_top` before a builtin
  runs and restores it after, as GNU R checks `R_PPStackTop` around a builtin.
  Early returns through `?` need nothing.
- A loop that keeps a fresh value each iteration brackets itself with
  `let m = cx.mark(); .. cx.release(m);`.
- The rule is GNU R's: keep what you hold across an allocation. A PROTECT in
  the C source is a `keep` in the port, one for one.
- Cost: a store and an add, the same as `PROTECT` (1.00 of GNU R in
  `crates/robigo-capi/bench`, ledger 2026-10-28).

##### 2.4 How the discipline is checked

- **Strict mode** (`ROBIGO_GC_STRICT=1`): the pin scan of Rust frames is off
  and gctorture poisons freed slots (core.md §3). A missing `keep` then fails
  at once. CI runs the conformance suite and the corpora slices in strict
  mode. This is also exactly the wasm32 configuration, where the machine stack
  cannot be scanned.
- **Release**: pins on. A counter `pin_saves` counts objects first marked in
  step 3 from a Rust frame. It is a health metric, reported by
  `ROBIGO_GC_STATS`.
- Cost of the pin scan: proportional to stack depth, about 300k words at 5,000
  nested calls (guess: under 1 ms). Measured in §10.

##### 2.5 VM register files, JIT frames, LLVM

- VM registers are value-stack slots: precise.
- **Native code holds an object only in a value-stack slot or an environment
  slot.** Machine registers hold unboxed scalars and data pointers derived
  from objects that a slot roots. Objects do not move, so a derived pointer
  stays valid across a helper that allocates. No stack maps and no
  statepoints, for Cranelift now and LLVM later. Today's JIT is already
  nearly this shape: its ABI is `(*mut u64, *mut VecView, *mut LoopCtx)`
  (jit.rs:134) and "any value" is a frame slot known by address
  (`JType::Dyn`, jit.rs:75-77).
- LLVM may keep only a derived form of a pointer in a Rust frame (a strength-
  reduced end pointer, a base minus a constant). For a rooted object that does
  not matter. For a pin-only object it can make the pin miss, which is why
  pins are the safety net and not the rule.

#### 3. Environments

Two representations behind one ENVSXP. Only ENCLOS is fixed (core.md §2,
`OFF_ENCLOS`).

```text
ENVSXP, 48 + 16*n bytes
+0   header   gp: GNU R's ENVFLAGS (FRAME_LOCK 1<<14, GLOBAL_FRAME 1<<15)
              extra: RDEBUG; ESCAPED (robigo, §4.3)
+8   attrib
+16  FRAME    pairlist of binding cells (GNU R's), or NULL
+24  ENCLOS
+32  HASHTAB  VECSXP of chains (GNU R's), or NULL
+40  aux      NULL | shape (slot frames) | version box (cell frames)
+48  n inline slots; n = LENGTH(shape)
```

##### 3.1 Cell frames: GNU R's representation

The global environment, namespaces, `new.env()`, `local()`, attached frames.
`n = 0`, so the object is today's 48-byte node and FRAME/HASHTAB are literally
GNU R's.

- envir.c is ported as written: `R_Newhashpjw`, `R_HashSet`, `R_HashGet`,
  `R_HashResize` at its growth rate, prepending in unhashed frames.
- Binding flags are GNU R's bits on the cell: `ACTIVE_BINDING_MASK` 1<<15 and
  `BINDING_LOCK_MASK` 1<<14 (Defn.h:1212-1213), MISSING in the low bits, the
  typed-cell tag in `extra` (Defn.h:462).
- `R_findVarLocInFrame` returns a real cell. `FRAME()` and `HASHTAB()` are
  field reads. `ls(sorted = FALSE)`, `as.list.environment`, `eapply` and
  `mget` get GNU R's order with no emulation.
- The base environment is the symbols' value cells, as GNU R (`IS_BASE`,
  `SYMVALUE`). A base function lookup that falls through everything ends in
  one load from the symbol.

##### 3.2 Slot frames: closure calls

A frame made by a compiled closure's call has `n` inline slots, one per formal
and per local the compiler found (cmp.R's `findLocals`). Its `shape` is a
VECSXP of the symbols in slot order, owned by the closure's `Code` and shared
by every call.

```text
slot (16 bytes):  key  symbol address | bits
                       bit 0     LOCKED
                       bits 1-2  MISSING (0, 1, 2: GNU R's values)
                       bits 3-5  kind: 0 object, 1 REAL, 2 INT, 3 LGL,
                                 4 active binding (val is the function),
                                 5 eager argument (object),
                                 6 eager argument (REAL), 7 (INT)
                  val  an object, or the scalar's bits
```

- The six bits are free because symbols are allocated 64-byte aligned in an
  arena of their own (§5).
- An unbound local has `val = R_UnboundValue`.
- The collector traces `val` when the kind is 0, 4 or 5; `n` comes from the
  shape. This is a new `ENVSXP` case in `Core::trace` (gc.rs:1349).
- **The VM reads and writes locals by slot index: `env + 48 + 16k`.** There is
  no register cache of variables and no flush. A scalar assignment writes the
  key (a constant) and the bits; if the slot held an object it loses a count.
  An object assignment goes through `Sexp::store` (barrier bit test, counts).
- Reading a slot as an object boxes an unboxed scalar and stores the box back:
  `R_expand_binding_value` (memory.c:4377). That one function serves `get`,
  `as.list(env)`, a promise of the variable and C's `findVar`, which is what
  #276 (C writes to purrr's index) needs.
- Lookup by name in a slot frame (AST evaluator, `get`, C): scan the shape; a
  shape has a small hash when it has more than 8 symbols.
- A variable the compiler did not see (`assign("z", 1)`, `eval` in the frame)
  goes to the FRAME pairlist as a GNU R cell, prepended. A slot frame with a
  non-NULL FRAME is "extended"; fast paths test that word.
- **Order.** GNU R lists a call frame's later definitions first, then the
  matched arguments (today's `Frame::nbase`, env.rs:84-90, emulates it). For
  most closures the order of first definitions is fixed by the code and the
  `Code` records it. Where it depends on the path taken, the compiler emits a
  defining store that records a rank in the key's high 16 bits (guess: under
  5% of closures).
- `FRAME()` on a slot frame builds a pairlist snapshot. No package imports
  FRAME (core.md §2). A varloc of a slot is the slot's address.
- Frames the AST evaluator makes (bootstrap, uncompiled closures) are GNU R
  cell frames. The two kinds mix freely on an ENCLOS chain.

Size: a three-formal call frame is 96 bytes in one allocation. Today's
`EnvData` is 240 bytes (env/trace.rs:78) plus the bindings' buffer beyond four
(env.rs:60).

##### 3.3 Variable lookup from compiled code

- Own locals: slot index, fixed at compile time.
- Free variables in enclosing closure frames: an inline cache
  `{hops, shape, slot}` per `GetVar` site. Each hop checks that the frame's
  shape is the cached one and that it is not extended: two loads and two
  compares. This is hidden-class caching; R makes it easy because every call
  of a closure has the same shape.
- Variables found in cell frames: the cache holds the binding cell, as GNU R's
  byte code does. A removed binding leaves `R_UnboundValue` in the cell, which
  the cache sees as a miss.
- From the global environment along the search path: GNU R's `R_GlobalCache`
  (envir.c:659, 707, 747, 1183), stored per symbol (§5: symbol word +40) and
  flushed per symbol when a `GLOBAL_FRAME` frame changes (envir.c:650). It
  replaces `Interp::fun_cache` (interp.rs:223) and the search epoch for
  lookups.

##### 3.4 Version counters and guards

Today a guard asks "does `sym`, looked up as a function from `env`, still
resolve to base?" and memoizes on (search epoch, env id, `fn_version`)
(compile.rs:556, vm.rs:2570). Misses on fresh frames cost about 5,000
instructions per `calls.R` iteration (perf-vm ledger, 2026-11-10).

- A slot frame is skipped by its shape. If `sym` is not in the shape and the
  frame is not extended, it cannot bind `sym`. That is known when the site is
  compiled; at run time it is the "not extended" test. The fresh-frame miss
  disappears.
- A cell frame gets a **version box** on demand: a 16-byte RAWSXP in `aux`
  with a counter bumped when a binding that could hold a function changes. A
  memo holds the box (a strong reference to a tiny object) and the version it
  saw. Because the memo holds the box and not the environment's address, a
  freed environment whose address is reused cannot satisfy a stale memo.
- A locked namespace with locked bindings cannot change: the guard is static
  (perf-vm ledger, "guards in namespace code").
- Rule for every cache in G3: **key on an object the cache keeps alive, or on
  a counter that never repeats. Never on a bare address.** Today's caches use
  `Weak` for this (compile.rs:633, interp.rs:77); a tracing heap has no cheap
  `Weak`, and address reuse was already a bug once (memory ledger, tracemem).

##### 3.5 `<<-`

`sym <<- v` is a free-variable store: the same `{hops, shape, slot}` cache,
then a slot write, or `Sexp::store` into a cell. `x[i] <<- v` is the in-place
protocol on the enclosing binding, with `commit_links()` first (§1.5). This is
the `<<-` idiom of `calls.R` (1.39x GNU R today).

##### 3.6 What this deletes

`env.rs`'s `Frame` (SmallVec entries, index, mask), `env/table.rs`,
`env/trace.rs` and `reachability.rs` (cycle tracking for `Rc`),
`Env::release_self_cycle` (env.rs:482), `autogc.rs`, `SEARCH_EPOCH` and
`R_EPOCH` as thread-locals (env.rs:118-130).

#### 4. Closures, promises, argument matching, contexts, unwinding

##### 4.1 Closures and code

```text
CLOSXP   +16 FORMALS  +24 BODY  +32 CLOENV  +40 aux: Code or NULL
LANGSXP  +16 CAR  +24 CDR  +32 TAG  +40 aux: Code compiled for this expression, or NULL
Code     64 bytes, type BCODESXP with robigo's layout; C never sees it
         +16 consts  VECSXP: constants, shapes, inline-cache objects (traced)
         +24 source  the function expression or body
         +32 shape   +40 aux: next specialization or NULL
         +48 *mut CodeData (a Rust allocation: ops, cache data)  +56 release fn
```

- BODY is always the AST, so `BODY()`, `R_ClosureExpr` and deparse need no
  unwrapping.
- The `function` special copies the `Code` from the `function` call's `aux`
  into the new closure's `aux`: closures made in a loop share code, as today
  (jit/closure.rs:32-33).
- `eval()` of a language object caches its compiled code in that object's
  `aux`. This replaces the `CACHE` thread-local that never evicts (vm.rs:289;
  design.md §5.2): the code dies with the expression.
- `CodeData` and its machine code are freed by the sweeper through the release
  function, outside the collection, as external data is today (core.md §3).
- `SET_BODY`, `SET_FORMALS` and `SET_CLOENV` from C clear the closure's `aux`.

##### 4.2 Promises

```text
PROMSXP  +16 PRVALUE (R_UnboundValue until forced; raw bits when tagged)
         +24 PRCODE   the expression, always
         +32 PRENV    NULL once forced
         +40 aux      the Code that holds its compiled thunk, or NULL
         gp bits 0-1 PRSEEN, bits 2-15 thunk index
         extra bits 4-5 PROMISE_TAG (REAL, INT, LGL): GNU R's
                  IMMEDIATE_PROMISE_VALUES (Defn.h:1178)
```

- An unboxed value is boxed when read as an object
  (`R_expand_promise_value`). Today's `PromiseVal::Scalar*` (value.rs:636) is
  this idea with a second representation; in G3 it is GNU R's own.
- "restarting interrupted promise evaluation" (PRSEEN 2) is as ported.
- **Eager arguments.** Where today's analysis allows the caller to evaluate an
  argument (`Code::strict`, perf-vm ledger "caller-forced promises"), G3
  writes the value into the callee's slot with kind 5, 6 or 7 and allocates no
  promise. R cannot tell: forcing gives the value, `missing()` reads the
  MISSING bits, `substitute()` takes the expression from the call. When
  something needs the promise as an object (C's `findVarInFrame`, or the frame
  outliving its call), the slot is turned into a forced promise then
  (`PRVALUE` set, `PRENV` NULL, `PRCODE` from the call site's matching plan).
  This generalizes `BIND_MISSING_DEFAULT` and `interp::materialize_default`
  (core.md §6, "Constant formal defaults") and removes the gap recorded there:
  the promise C sees is forced when R has read it. It also closes #277.
- A constant default is bound the same way.

##### 4.3 Calls

- **Matching** is match.c's three passes. Per call site the result is cached
  as a plan (argument i goes to slot j, what goes to `...`, which formals are
  missing), valid while the callee's FORMALS is the same object. This is
  today's `CallSite` (compile.rs:636).
- `...` is a DOTSXP of cons cells, as GNU R. rlang walks it with `CAR`/`CDR`.
  One 48-byte node per dots argument per call (measure).
- **Frame exit** is `R_CleanupEnvir` (eval.c:2132): when the frame has no
  references left (its own cycles discounted), clear promises, dots and
  bindings and set ENCLOS to the empty environment.
- **Recycling.** On top of that, if the frame is unreferenced and its ESCAPED
  bit is clear, its memory goes back to a LIFO list for the next call.
  ESCAPED is set whenever the frame's address is handed out as a value with no
  counted edge: `environment()`, `sys.frame()`, `parent.frame()`,
  `sys.function`, `R_GetCurrentEnv` and the `rho` of `.External2`. Promises
  cleared at exit are recycled the same way. This is what "arena frames with
  escape promotion" (#523 item 3) wanted and could not have under `Rc`. It is
  a measured option (§10): the alternative is small young collections.
- A frame that survives its call turns its eager slots into promises at exit,
  while the call is still at hand.

##### 4.4 The context stack

One stack of `#[repr(C)]` records in the isolate. It replaces
`Interp::frames: Vec<Context>` (interp.rs:51, 86), `ccode_calls`,
`srcref_saved`, capi's `CONTEXTS` and `errors::Ctx`.

```rust
###[repr(C)]
pub struct Rcntxt {             // R: Defn.h RCNTXT; robigo's Defn.h declares this struct
    callflag: i32, evaldepth: i32,
    call: Value, callfun: Value, cloenv: Value, sysparent: Value,
    promargs: Value,            // NULL until asked for; built from args_base/nargs
    args_base: *mut Slot, nargs: u32, flags: u32,   // browserfinish, generic, bc
    conexit: Value, handlerstack: Value, restartstack: Value,
    srcref: Value, return_value: Value,
    cstacktop: i32,             // R_PPStackTop
    vstack_top: *mut Slot,      // GNU R's nodestack and bcprottop
    vmax: *mut u8,
    cjmpbuf: *mut JmpCtx,       // NULL for a context Rust code opened
    cend: Option<extern "C" fn(*mut c_void)>, cenddata: *mut c_void,
}
```

- Its `Value` fields are precise roots, as memory.c scans `R_GlobalContext`.
- `promargs` is built only when `sys.call`, `match.call`, `sys.function` or C
  asks. GNU R conses it on every call.
- `R_HandlerStack` and `R_RestartStack` are isolate fields holding errors.c's
  lists; a context saves two words. This removes the `Rc` clones the ledger
  found on `R_UnwindProtect`'s normal path (core ledger, "caught-error
  benchmark").
- context.c (`begincontext`, `endcontext`, `R_run_onexits`, `R_sysframe`,
  `R_syscall`, `R_sysfunction`, `R_restore_globals`) is ported onto it.

##### 4.5 `Flow` and unwinding

- Rust code never jumps (core.md §5). That stays.
- `Flow` becomes a zero-sized token and the exit itself moves into the
  isolate:

  ```rust
  pub struct Unwind;                      // 1-ZST: Result<Value, Unwind> is one word
  pub type RResult<T> = Result<T, Unwind>;
  struct Pending { kind: ExitKind,        // Error, Return, Break, Next, Restart, Interrupt, Exec
                   target: *mut Rcntxt, mask: i32,
                   value: Value, condition: Value, call: Value }   // traced
  ```

  Today `Flow` carries a `Value` and an `Env` or a boxed error (error.rs:20),
  so every `RResult<Value>` returns through memory, and its `Value`s would be
  unrooted while `on.exit` code runs.
- `Flow::error(msg)` and `Flow::error_call(call, msg)` keep their names. They
  fill `Pending` and return the token.
- This is capi's existing model (`errors::Unwind`, `errors::Res`, the pending
  condition), so the runtime and capi share one error type and capi's `FLOWS`
  token table (bridge.rs:88) goes.
- C ↔ Rust ↔ C: unchanged from core.md §5. A Rust function returns
  `Err(Unwind)`. The `extern "C"` wrapper that C called turns it into a
  `longjmp` to the innermost C context. The C trampoline turns the jump back
  into `Err(Unwind)`. Each context an exit passes runs its `conexit` and
  restores what `R_restore_globals` restores. `Pending.target` is GNU R's
  jump target, so `return` from a promise forced under a `.Call` finds its
  frame as in GNU R.

#### 5. Strings

- `RStr`/`RString` (286 uses) become `RChar`, a `Copy` newtype over a CHARSXP.
  A STRSXP element is 8 bytes (16 today).
- `c.is_na()` is a pointer compare with the static `NA_STRING`. `c.bytes()` is
  `&[u8]`. `c.enc()` reads the gp bits; the rules are `robigo_core::chars`
  (core.md §4).
- **`&str` views.** `c.as_str() -> Option<&str>`: if ASCII (one bit test), the
  bytes are the string. Otherwise it is valid only if the bytes are valid
  UTF-8 and the mark allows it. A UTF-8 mark does not prove validity in GNU R,
  so the answer is computed once and cached in two bits of the CHARSXP's
  `extra` word.
- **Translation** happens where GNU R's C translates. `c.to_utf8(cx)` is
  `translateCharUTF8`: borrowed for ASCII and UTF-8, converted for Latin-1 and
  native, an error for bytes. `c.to_native(cx)` is `translateChar`. A builtin
  ported from C calls them where the C calls them.
- **Layout additions** (the CHARSXP header has a spare u32 at +12, core.md
  §2):

  ```text
  CHARSXP +8  hash u32   +12 symbol id + 1 (0: no symbol has this name)
  SYMSXP  64 bytes, 64-byte aligned, immortal, in the symbol arena
          +16 PRINTNAME  +24 SYMVALUE (base's binding)  +32 INTERNAL
          +40 aux: global-cache cell (§3.3)
          +48 id u32  +52 hash u32  +56 spare (the isolate's value index later)
  ```

  `install(CHARSXP)` is then two loads with no hashing. That is the path of
  `get("x")`, `e[["x"]]`, `x$name` with a computed name, `attr(x, "names")`,
  `as.name` and `do.call("f", ..)`.
- `Sym` keeps its name as a newtype over the symbol's address. Today it is a
  `u32` index (sym.rs:13); `Sym.0` and `Sym::index()` uses change, the 1,054
  type uses do not.
- **Hashing.** `unique`, `duplicated` and `match` on strings hash the CHARSXP
  pointer and compare pointers when both strings are cached and their
  encodings allow it (unique.c:207 and `sequal`). No string bytes are read.
- **`paste`** builds bytes in a scratch buffer and interns once per result
  with `mkCharLenCE`. Interning measures at 0.59 of GNU R on a hit and 0.82 on
  a miss (core ledger, 2026-10-28). Against today's uninterned `Rc<str>` it
  may cost more per fresh string (§10 measures it). Always interning stays:
  packages compare CHARSXP pointers (core.md §4).

#### 6. Vectors and ALTREP

- **Storage** is core.md §2's: inline (the default; a scalar is one 48-byte
  object), EXTDATA (a foreign writable buffer, `allocVector3`'s custom
  allocator), ALTREP (compact sequences, wrappers, deferred strings, foreign
  read-only buffers, package classes).
- The bridge's uses of EXTDATA (`HOST_OWNED`) and the HOST proxies go. With
  them go `Core::hold`/`watch`/`held_roots`/`host_trace` (gc.rs:144-178) and
  the header bits TWIN, WATCHED, HOST_OWNED and HOST. G3 reuses two freed
  bits: `WRITING` (§1.4) and `ESCAPED` (§4.3).
- **Compact sequences** are robigo's own ALTREP classes
  (`crates/robigo-capi/src/compact.rs` moves to the runtime). `for` reads them
  with no object per element.
- **Package ALTREP** is GNU R's: each operation calls the class's method
  through the C trampoline. The copies, twins and deferred wrappers of core.md
  §6 (#422, #468) and `robigo_runtime::altrep` go. `length(x)` calls `Length`
  only, which closes the "known gap" of core.md §6.
- **Growth.** `x[n + 1] <- v` is `EnlargeVector` (subassign.c:140): the
  GROWABLE bit and TRUELENGTH as GNU R sets them, since
  `.Internal(inspect)` prints `tl`. Beyond that, robigo grows in place
  whenever the object's allocation has room. A size class has slack, and a
  large vector's chunk can reserve address space past its end. The capacity
  comes from heap metadata, never from TRUELENGTH, so R sees GNU R's numbers
  and the loop is amortized constant instead of GNU R's 5% steps. (Guess:
  reserving 2x for chunks above 1 MiB costs no resident memory. Measure RSS.)
- **Attributes** are the pairlist at +8. attrib.c is ported once, on `Value`,
  in the runtime; capi's `attrib.rs` (773 lines) becomes wrappers over it. The
  same for `coerce.rs`, `list.rs`, `objects.rs`, `envir.rs`, `sort.rs` and
  `serialize.rs` in capi: after G3, robigo-capi holds export wrappers,
  trampolines, headers, loading, the graphics engine and `appl`, and no R
  semantics of its own.
- The S4 bit and OBJSXP are header state (core.md §13), which unblocks G6's
  first ask.

#### 7. VM and JIT on core objects

##### 7.1 VM

- Registers are value-stack slots (16 bytes, typed, §2.2). Variables are
  environment slots (§3.2).
- The dispatch loop keeps in machine registers: the pc, the frame's slot base,
  the environment pointer and `cx`.
- Scalar arithmetic reads tags and bits from slots, as `scalar()` does today
  (vm.rs:93), with a path for a boxed length-1 vector without attributes.
- A VM-to-VM call pushes a frame on the value stack, allocates or recycles the
  environment, binds arguments by the site's plan, and opens a context. An
  unboxed scalar result returns in a slot (today's `Interp::unboxed_ret`,
  interp.rs:229, becomes the normal return slot).

##### 7.2 What carries over (perf-vm ledger, "For G3")

| technique | in G3 |
|---|---|
| call-site function lookup through local frames | shape tests, then the cell or global cache |
| pure-closure inlining | unchanged; arguments come from slots |
| caller-forced promises | eager slots (§4.2), no promise object |
| index promises forced in R's order | unchanged |
| guard memoization | shapes and version boxes (§3.4) |
| static per-site rejection memos | unchanged |
| `%*%` row vector as dot products | unchanged |
| nested pure-call inlining, whole-closure native code, loop summaries | unchanged in design; re-targeted |

Redone, not ported: the `SmallVec` register and slot buffers, `set_var` and
`own_var`'s `Rc::get_mut` tricks, `Vars` and `flush`, the layout tuning of
`run_with`.

##### 7.3 Inline caches

They live in the `Code`: objects in `consts` (traced), plain data in
`CodeData`.

| site | cache | valid while |
|---|---|---|
| call | callee `Code`, matching plan, inline plan | `callee.aux == Code` and `callee.FORMALS` unchanged |
| free variable | hops, shapes, slot | each frame's shape matches and it is not extended |
| global variable or function | binding cell | the cell's value is not `R_UnboundValue` |
| guard | version box and version | the box's version is unchanged |
| S3 dispatch | class attribute object, method closure | same class object and the method's cell unchanged |

The S3 row is new. Dispatch costs 18.8k instructions today (perf-vm ledger,
2026-11-10: `lookup_method` 840, interning 271, allocator about 12%). Keying
on the identity of the class attribute's STRSXP is exact, because a class
vector that is the same object has the same contents while the cache holds it.

##### 7.4 JIT

- Native code reads objects by header offset: type at `+0`, flags at `+1`,
  count at `+4`, attributes at `+8` compared with NULL's static address,
  length at `+16`, data at `x + 32`. A vector access guard is two byte loads
  and one word compare.
- `VecView` (jit.rs:121) marshalling at entry goes; native code derives the
  data pointer itself.
- Scalars in loops stay in machine registers and are written back to slots at
  exits, as today. Writing a scalar to an environment slot is two stores: the
  key is a constant (symbol address and kind), the bits are the value. Entry
  guards check that the slot is neither locked nor active.
- In-place vector writes check `refcnt <= 1` at entry and run after
  `commit_links()`. Native code runs no R code, so the check holds for the
  whole run.
- **Deoptimization** is unchanged: a loop leaves at op granularity with state
  written back; a call tree is abandoned and replayed by the VM
  (jit/closure.rs:12-19). Results stay the VM's.
- **Rooting**: §2.5. No stack maps.

#### 8. The isolate record

```rust
// crates/robigo-runtime/src/isolate.rs
###[repr(C)]
pub struct Interp {                  // the isolate: one heap, one evaluator, one thread
    // hot: fixed offsets for the VM and native code
    vstack_top: *mut Slot, vstack_limit: *mut Slot, links_committed: *mut Slot,
    ctx_top: *mut Rcntxt, pp_top: i32, eval_depth: i32,
    visible: bool, interrupts_pending: bool, gc_inhibit: u32,
    frame_free: [Value; 8],          // recycled frames by size (§4.3)
    core: robigo_core::Core,         // the heap, inline
    pending: Pending,
    global: Env, base: Env, base_ns: Env, empty: Env, ns_registry: Env,
    handler_stack: Value, restart_stack: Value, srcref: Value,
    // everything that is a thread_local! today
    numerics, rng, options, conns, s4, dlls, warnings, locale, tz, ...
    ext: [*mut (); 4],               // robigo-capi's tables (state::Tables), opaque here
}
```

- **What moves in:** all 69 `thread_local!`s of the runtime (the symbol
  interner, `SEARCH_EPOCH`, `R_EPOCH`, `NEXT_ENV_ID`, the bytecode `CACHE`,
  the compact-sequence registries, `HASH_TABLES`, the JIT modules, locale and
  time-zone state, ...), capi's remaining ones (core ledger, 2026-11-10:
  `PENDING`, `SCRATCH`, `CONTEXTS`, the bridge's 42, which die) and `Core`.
- **How code reaches it:**
  - the evaluator, the VM and builtins get `cx: &mut Interp`, as today;
  - native code gets it as an argument and keeps it in a callee-saved
    register;
  - a C entry point (an export wrapper, a signal handler) reads the one
    thread-local, `CURRENT: Cell<*mut Interp>`, once, and passes it down;
  - a helper with no `cx` (the constructors of §1.6, `Flow::error`) reads the
    thread-local. In the executable that is one `%fs`-relative load. Today's
    reads cost about 25 of 5,794 instructions per call (perf-vm ledger,
    2026-11-10).
  - `scripts/lint.sh` fails on a new `thread_local!` in the runtime, capi or
    core.
- **capi's per-isolate exports.** `R_GlobalEnv` and the others stay exported
  variables mirroring the record until isolates run as threads (core.md §10
  A.2); then robigo's headers turn them into accessors. Not G3 work.
- **The libR.so revisit** (core.md §12 item 15). The two measured blockers
  were `__tls_get_addr` on every thread-local and the out-of-line allocator
  shim (core ledger, 2026-11-10). After G3 the first is one read per C entry.
  The second shrinks without being fixed: objects, frames, promises and
  argument runs no longer come from the Rust allocator. Re-run that ledger
  entry's experiment after the flip and keep its rule (no benchmark slower).
  Expect the C API microbenchmarks to decide it (guess).

#### 9. Migration

##### 9.1 The coordinator's plan, assessed

| step | verdict |
|---|---|
| 1. council-reviewed design | yes |
| 2. measured prototype | yes, with the questions and exit criteria of §10 |
| 3. narrow `Value`'s interface on main; isolate record | yes, and widen it: the whole handle API of §1, not only private variants and `view()` |
| 4. one swap on a long-lived branch | no |
| no mixed heap | agreed |

- **"The swap is local to `value.rs`" does not hold.** Five things leak
  through any `Value` interface:
  - ownership: 142 `make_mut` and 156 `strong_count`/`ptr_eq`/`get_mut`/`Weak`
    sites encode "`Rc` counts every holder";
  - strings: `Option<RString>` patterns at 620 `Value::Str` sites;
  - calls and pairlists as `Vec<Arg>` (279 `Value::Lang` sites, `Lang.args`);
  - environments: 329 `borrow()` sites over `RefCell<Frame>`;
  - the evaluator's in-place protocol, the VM's register caches and the JIT's
    ABI, which are rewritten, not swapped.
- **A long-lived branch is the largest risk in the plan.** Main takes dozens
  of commits a day across the same files. A branch that rewrites the evaluator
  and touches every builtin has no green ratchet for weeks, and each rebase
  re-converts whatever main added.
- **No mixed heap: agreed, for the reasons the core ledger records.** A value
  whose attributes can point into either heap needs cross-heap roots and a
  cross-heap cycle collector. That is the bridge (core.md §6, "One mark over
  both heaps") and the source of #278, #189, #304, #167/#389 and #217.

##### 9.2 The alternative: one API, two backends, both on main

```text
crates/robigo-runtime/src/value/mod.rs    the API: Value, View, Real, .., Unique, Args, RChar, Sym, Env
                              value/rc.rs  backend A: today's Rc model (default)
                              value/gc.rs  backend B: core objects (feature "g3")
crates/robigo-runtime/src/g3/             eval.rs apply.rs promise.rs ctx.rs env.rs unwind.rs
                                          links.rs vm.rs compile.rs jit/   (feature "g3" only)
```

- Builtins, printing, formatting, deparse, coercion, attributes and dispatch
  are written against the API and compile under both backends with no `cfg` at
  their call sites.
- The old evaluator (`interp.rs`, `vm.rs`, `compile.rs`, `jit*`, `env.rs`)
  stays as it is for backend A and is not converted.
- `robigo` (backend A) is what users, the ratchets and the perf gate see until
  the flip. `cargo build --features g3` builds the new engine from the same
  tree.
- This is "a new runtime grown beside the old one" for the evaluator's 25-30k
  lines and "conversion in place" for the other 85k. It is not a mixed heap:
  each binary has exactly one object model.

How the pieces verify:

- A converted builtin module is checked at once by today's ratchets under
  backend A. Its behaviour does not depend on the backend.
- The `g3` build has ratchets of its own: `tests/r/passing-g3/`,
  `tests/corpora/*/passing-g3/`. They start empty and only grow. CI builds
  both and runs the `g3` ratchet, in strict GC mode (§2.4).
- What backend A cannot check is exactly what is new in B: roots (`keep` is a
  no-op under `Rc`) and the link protocol. Strict mode and the differential
  oracle of §11 risk 1 cover them.

##### 9.3 How the builtins get converted

Mechanical first, by hand where it is semantic. A codemod built on `syn`
(already a dev-dependency of capi) rewrites, per module:

| pattern | count | becomes | how |
|---|---|---|---|
| `match v { Value::X(r) => .. }` | 4,272 | `match v.view() { View::X(r) => .. }` | codemod |
| `r.data` read | most of 1,873 | `r.slice(cx)?` or `r.std()` | codemod, then review |
| `r.attrs` | 245 | `r.attrib()` and the attrib.rs API | by hand |
| `Rc::make_mut(..)` | 142 | `cx.writable(..)` | by hand; check against the C source |
| `Value::X(Rc::new(Vector::new(v)))` | 188 | constructors | codemod |
| `Some(s)` / `None` on strings | about 620 | `c.opt()`, `c.is_na()` | codemod, then review |
| `l.fun`, `l.args[i]` | 279 | `call.fun()`, `call.args()` | by hand |
| `&Value` parameters | everywhere | kept until the flip, then `Value` | codemod at the end |

- One PR per module along the existing ownership lines
  (`docs/workstreams.md`). Each passes the module's corpus slice and
  `scripts/perf.py compare` (0.5%).
- `view()` over the `Rc` enum is the identity after inlining: it must show
  zero in the instruction counts, and the first converted module proves it
  (§10, E5).
- A count ratchet makes progress a number: the count of `Value::[A-Z]`
  patterns outside `value/` and the old evaluator, stored in a file, may only
  go down.
- Roots (`keep`) are added while converting, from the C source's PROTECTs.

##### 9.4 Order of work

All on main. Core changes land in robigo-core with its own tests; they do not
touch backend A.

1. **Prototype** (§10), in `experiments/g3`, a workspace of its own like
   `experiments/memory`. Decisions recorded in a new ledger,
   `docs/ledger/g3.md`.
2. **API freeze**: `docs/object-api.md` plus `value/mod.rs` and `value/rc.rs`.
   Three pilot modules converted (`arith.rs`, `paste.rs`, `subassign.rs`).
3. In parallel:
   - builtin conversion, module by module (many agents);
   - the isolate record: thread-locals into `Interp` (layout-independent; the
     coordinator's step 3);
   - core for G3: two-pass mark, value-stack and context roots, ENVSXP with
     inline slots, the tagged PROMSXP trace, static immortals, the symbol
     arena, `free_now` for recycled frames, removal of nothing yet.
4. **The `g3` evaluator**, in this order, each step raising the `g3` ratchet:
   AST `eval` and `applyClosure` on cell frames with `--no-base`; contexts,
   conditions and unwinding; lazy loading and the base library; the compiler
   and VM with slot frames; `.Call` through capi's exports called directly
   (in the `g3` build the bridge is not compiled and `Host` is gone); the JIT.
5. **Parity gates**: the `g3` ratchets contain main's; `perf.py` shows no
   benchmark slower than backend A and `calls.R` at or below GNU R; the capi
   corpus and the CRAN package suite are at or above main's; the leak tests
   are bounded; `1:2e8` runs in constant memory; byte strings round-trip (the
   G3 exit gate, design.md §5.6).
6. **Flip** the default. In the same week delete: `value/rc.rs`, the old
   evaluator, `bridge.rs` (5,971 lines), the runtime's `native/` conversion,
   `reachability.rs`, `autogc.rs`, `env/table.rs`, `env/trace.rs`,
   `altrep.rs`, core's HOST and hold machinery. Run the end codemods
   (`&Value` to `Value`, `clippy --fix` for `clone_on_copy`). Close #278,
   #277, #276, #189, #304, #167, #389, #217, #460 and #140 with tests.
7. Then: Rust ports of base natives that existed only to avoid the bridge's
   copy go, measured (core.md §8); libR.so re-measured (§8); generational
   collections.

The bridge dies at step 6, not before. Backend A needs it until then.

##### 9.5 What pauses on main now, and what continues

Pause:

- VM and JIT tuning that depends on today's layout: `run_with` register
  pressure, `SmallVec` buffers, `Rc` economy, frame recycling under `Rc`, the
  "next" list of the perf-vm ledger's 2026-11-10 entry.
- New bridge mechanisms (more twin kinds, more deferred shapes). Fix only what
  blocks a package ratchet.
- Any new `Value::<Variant>` match or `.data` access in a converted module
  (the count ratchet enforces it).
- `RString` extensions (already decided, core.md §4.1).

Continue:

- Conformance work, written against the API.
- robigo-core, capi's C surface, base natives, the package corpus.
- The parser, numerics, the REPL.
- S4/G6 design. It gets its header bit at the flip.

Change now: AGENTS.md's porting rule 5 and "Writing a builtin" are rewritten
for the API when it freezes.

#### 10. The prototype and the measurement plan

##### 10.1 Experiment A: runs on main today, no G3 code

Reorder the mark so that conservative words only pin (§2.1), on today's main.
Run: core and capi tests under gctorture; the capi corpus; the ledger's
finalizer stress script (2026-11-11: `.Call('pointer')` at depths 0..20 by
0..20) with the stub and the zeroing removed; the CRAN package suite.

- Decides: whether C frames can be pin-only, and so whether the scan-segment
  machinery goes.
- Pass: no capi or package regression; the stress script misses nothing in any
  tier with the stub and zeroing off.

##### 10.2 The prototype evaluator

`experiments/g3`: robigo-syntax's parser, robigo-core, and a minimal runtime
of 6-8k lines (guess): cons-cell AST, cell and slot frames, promises with
eager slots, contexts and unwinding, a register VM on the value stack, scalar
arithmetic and comparison, `[`, `[[`, `[<-` on atomic vectors, `c`, `list`,
`$` on environments, `<<-`, `UseMethod`, and enough of conditions to run
base's own `tryCatch` source. No JIT.

Benchmarks: `fib`, `binarytrees`, and the six idioms of
`benchmarks/suite/calls.R` separately (a default left to its promise, an
environment method, `<<-` state, S3 dispatch, `tryCatch`, `c(i, 2, 3)`).
Compared with today's robigo with its JIT off and with GNU R 4.5.0.

Metrics, in the perf-vm ledger's method: callgrind instructions per iteration
by N-against-2N differencing; cache simulation with estimated cycles
`Ir + 10·D1m + 100·LLm` (memory ledger); heap allocations per call; peak RSS;
longest pause.

| # | question | variants | decides |
|---|---|---|---|
| Q1 | frame representation | inline slots; GNU R pairlist frames; today's VM | §3.2 |
| Q2 | frame reuse | LIFO recycling with ESCAPED; young collections every 2 MiB (the bake-off's knee, memory ledger); GNU R's trigger only | §4.3 |
| Q3 | rooting cost | value stack and `keep`; the same with the pin scan on; register clearing against liveness bitmaps | §2 |
| Q4 | error return | one-word `RResult` with `Pending`; today's `Flow` by value | §4.5 |
| Q5 | arguments | eager slots; a forced PROMSXP per argument | §4.2 |
| Q6 | link protocol | cost of the compare on return; cost of commits in the `<<-` idiom | §1.5 |
| Q7 | strings | `paste`, `match`, `unique`, `sprintf` loops on core CHARSXPs against today's `RString` | §5 |
| Q8 | reaching the heap | `cx.alloc` against the thread-local in a constructor | §8 |

##### 10.3 Exit criteria: the rebuild starts when all hold

- **E1.** Every `calls.R` idiom at or below 1.0x GNU R's instructions in the
  prototype's VM with no JIT (today 1.10-1.83x).
- **E2.** `fib` and `binarytrees` at or below today's robigo with the JIT off,
  in instructions and estimated cycles; `binarytrees` peak RSS at or below
  today's.
- **E3.** The prototype passes its tests in strict GC mode, and the finalizer
  stress script misses nothing at any depth.
- **E4.** `ROBIGO_NO_INPLACE=1` (§11 risk 1) gives identical output on every
  prototype test.
- **E5.** The three pilot modules compile under both backends with no `cfg` at
  call sites, pass their corpus slices under backend A, and `perf.py compare`
  shows nothing slower than 0.5%.
- **E6.** `paste`/`match`/`unique` on CHARSXPs at or below today's
  instructions, or the difference explained and accepted in the ledger.

If E1 fails, the frame, promise or call design is wrong, and that is found
before any of the 115k lines move. That is the purpose of doing it first.

#### 11. Risks and open questions, ranked

| # | risk | what resolves it |
|---|---|---|
| 1 | The link protocol misses a case: an in-place write is visible through an uncounted reference. Silent wrong answers. | A differential oracle. `ROBIGO_NO_INPLACE=1` makes every `MAYBE_SHARED` true. In-place modification is unobservable, so any output difference between the two modes over the corpora is a bug. `tracemem` cases against GNU R check the other direction (copies where GNU R copies). |
| 2 | Missing precise roots. | Strict mode in CI (§2.4); `pin_saves` in release; `keep` ported from the C source's PROTECTs. |
| 3 | G3's VM is slower than today's on scalar code (barriers on slot stores, typed slots, counts). | E1 and E2 before the rebuild; the `perf.py` gate at the flip. |
| 4 | The conversion stalls or the two backends drift. | Two numbers per week: the `Value::` count going down and the `g3` ratchet going up. If the API needs a `cfg` at a call site, the API is wrong: fix it there. |
| 5 | Pin-only C frames break a package that works on GNU R by luck. | Experiment A. Fallback: C segments stay strong roots, as today. |
| 6 | Interning makes string-building slower than today. | Q7. If so, measure uncached CHARSXPs for intermediates (GNU R's `IS_CACHED` paths exist) before accepting. |
| 7 | An address-keyed cache meets address reuse. | The rule of §3.4; the `g3` ratchet under gctorture, which reuses slots at once. |
| 8 | GNU R's counts copy where `Rc` did not, and some benchmark gets slower. | `perf.py` at the flip, by benchmark. The fix is porting `R_CleanupEnvir` exactly, never extra decrements, which `inspect` and `tracemem` would show. |
| 9 | A package's ALTREP method evaluates R code under `slice()`. | The "no evaluation" flag raises a clear error; the package suite (vroom, arrow, cli) finds it. |
| 10 | The static immortal objects complicate isolates, since a symbol's value and cache words are mutable. | Per-isolate arrays indexed by the symbol's id (§5, the spare word). Not G3; the layout leaves room. |
| 11 | The value stack's fixed reservation, and deep recursion messages. | The existing conformance cases for "evaluation nested too deeply" and C stack usage, in the `g3` ratchet. |
| 12 | LLVM hides the only pointer to a pin-only object in a derived form. | Strict mode means nothing relies on pins. No further action. |
| 13 | C writes through `REAL(x)` on a shared object while Rust holds a slice. | Parity with GNU R. Rule: no slice across `.Call` beyond what GNU R's C holds. |
| 14 | The API on backend A slows main. | E5, then every conversion PR's `perf.py compare`. |

Open questions for the coordinator, with my answer:

- Should `Root` be an RAII guard instead of `keep`? No. A guard that borrows
  `cx` blocks `cx`; one that does not needs the thread-local in `Drop`. The
  dispatcher's reset is free and matches how the C sources are written.
- Should release builds scan Rust frames for pins at all, given strict CI?
  Yes, until `pin_saves` has been zero across the package suite for a release
  cycle. Then measure the scan's cost and decide.
- Should hashed environments get a faster table than GNU R's chained one?
  Not in G3. Binding-cell caches make the table cold, and its order is
  observable.

#### Appendix: changes to robigo-core that G3 needs

| file | change |
|---|---|
| `gc.rs` `collect_needing` | pins after weak processing (§2.1) |
| `gc.rs` roots | the value stack (tag-aware), the context stack, the isolate's fields |
| `gc.rs` `trace` | ENVSXP with inline slots; PROMSXP with PROMISE_TAG; typed binding cells; `Code` objects |
| `gc.rs`, `lib.rs` | static immortal objects: the barrier's slow path, `incref`/`decref` unchanged (IMMORTAL) |
| `heap.rs` | the symbol arena (64 bytes, 64-byte aligned); `free_now(x)` for recycled frames and promises |
| `lib.rs` | `alloc_env(shape, enclos)`; `extra` bits WRITING and ESCAPED; CHARSXP word +12; slot and key constants |
| new `stack.rs` | the value stack: reservation, `Slot`, scanning |
| removed at the flip | HOST, EXTDATA's host owner, `hold`, `watch`, `held_roots`, `host_trace`, TWIN, WATCHED, HOST_OWNED; the scan segments if experiment A passes |


### Opus

### G3: robigo's runtime rebuilt on robigo-core — design (Claude Opus 5.5)

Base: main `4cff0ceb`. Read-only review: `AGENTS.md`, `docs/architecture.md`,
`docs/design.md` §3/§5, `docs/core.md` (whole), `docs/ledger/perf-vm.md`
(top entry, "For G3"), `docs/ledger/core.md` (2026-11-11, 2026-11-10,
2026-10-28 "conservative scan limited to C frames"), `docs/ledger/memory.md`,
the code named in the brief, and GNU R 4.5.0's `eval.c`, `envir.c`,
`subassign.c` and `Defn.h`. Where I guess, I say so.

#### 0. Summary of positions

1. **`Value` goes away as a type that owns anything.** Safe runtime code
   holds `Ref<'g>`: an 8-byte `Copy` pointer to a core object, branded with
   the lifetime of a *shared* borrow of the isolate (`&'g Cx`). Typed views
   (`View<'g>`, `Vector<'g, f64>` → `&'g [f64]`) are for matching and for
   reading. A function that can run R code or C code takes `&mut Cx`, so
   **the borrow checker kills every unrooted `Ref` at every R or C
   callback**. A value that has to survive a callback lives in a precise
   root: an argument window, an output slot, a `RootScope` or a `Global`.
2. **Rooting is a hybrid with one clear rule.** The collector scans
   conservatively only (a) C segments, as today, and (b) the *innermost
   Rust segment*: the frames below the most recent entry into R code.
   Outer Rust frames are suspended at that entry and are not scanned.
   This is sound because of (1). It is deterministic at an explicit
   `gc()`, because `do_gc` opens a segment that holds nothing. Allocation
   needs only `&Cx`, so builtins (60k lines) pay no `PROTECT` tax between
   callbacks. That is where GNU R's C needs most of its `PROTECT`s, and
   there conservative scanning gives the safety for free.
3. **Copy-on-modify is GNU R's REFCNT protocol, exactly**, including the
   byte-code interpreter's deferred link counting for stack values
   (`eval.c:991-1050`, `INCLNK_stack_commit`). `Rc::make_mut` and the
   "take arguments by value" rule (porting contract §5) are replaced by
   `MAYBE_SHARED(x)` tests at the places GNU R's C makes them.
4. **Environments are variable-size objects with inline slot frames.** A
   closure call allocates one object for its frame. A shape (the closure's
   compiled code) maps symbols to slots, so the VM reads and writes
   variables by index. Per-binding flags and GNU R's unboxed binding
   cells (`BNDCELL_TAG`, `Defn.h:462`) live beside the slots. Base
   bindings sit in a per-isolate table indexed by symbol id, and the
   global cache goes into that table too.
5. **Migration: grow a new runtime crate (`robigo-rt`) on main, beside
   the old one, with its own ratchets. Use no long-lived integration
   branch and no `Value`-narrowing step on main.** The coordinator's step 3
   would rewrite ~4,900 sites into an interface that cannot express the
   target. Their step 4 is a big-bang swap with no signal to climb for
   weeks. The new crate is hill-climbable from its first day, conflicts
   with nobody, and is flipped to the default when its ratchets contain
   the old ones and it is faster on every benchmark. The flip PR deletes
   the bridge. I agree with the coordinator's "no mixed heap" argument for
   persistent cross-heap references. I describe a narrow, optional
   exception for booting (leaf builtins through transient conversion) in §9.

---

#### 1. The Rust-side object API (council item 24)

##### 1.1 Layers

| layer | type | who uses it | safety |
|---|---|---|---|
| raw | `robigo_core::Sexp` (`lib.rs:140`) | robigo-core, robigo-capi, the evaluator core (`rt/eval`, `rt/vm`) | `unsafe fn`s; rules in core.md §3 |
| branded | `Ref<'g>`, `View<'g>`, typed views | builtins, printing, deparse, coercion, attributes: everything else | safe; the compiler checks rooting across callbacks |
| rooted | `Root<'s>`, `Out<'s>`, `Args<'a>`, `Global` | anything that holds an object across a callback, or longer | safe; precise roots |

The module that defines the branded layer (`crates/robigo-rt/src/obj/`) is
a deep module. Its interface has about six types, and it hides the layout
completely: no builtin sees an offset, a flag bit or a refcount field.

##### 1.2 Types

```rust
// crates/robigo-rt/src/obj/mod.rs
/// The isolate: heap, interpreter state, VM stack, contexts (§8).
/// `!Send + !Sync`. Allocation needs `&Cx`; running R or C code needs `&mut Cx`.
pub struct Cx { /* §8 */ }

/// A live object, valid while the shared borrow `'g` of the isolate lasts.
###[derive(Clone, Copy, PartialEq, Eq)]
###[repr(transparent)]
pub struct Ref<'g> { p: Sexp, _g: PhantomData<&'g Cx> }

/// An interned symbol: immortal, so unbranded. `id()` is dense (§3.6).
###[derive(Clone, Copy, PartialEq, Eq, Hash)]
###[repr(transparent)]
pub struct Sym(Sexp);

/// Precise roots.
pub struct RootScope<'s> { stack: *mut RootStack, base: u32, _s: PhantomData<&'s mut ()> }
###[derive(Clone, Copy)] pub struct Root<'s> { slot: *const Sexp, _s: PhantomData<&'s ()> }
pub struct Out<'s> { slot: *mut Sexp, _s: PhantomData<&'s mut ()> }   // an output register
pub struct Args<'a> { base: *const u64, tags: *const u8, n: u32, names: *const Sym, _a: PhantomData<&'a ()> }
pub struct Global { idx: u32 }  // a slot in Cx's global root table; for Cx fields and caches
```

- `Ref<'g>::view(self) -> View<'g>` is the only match point (one byte
  load of `type` at +0, plus the ALTREP bit for vectors):

```rust
pub enum View<'g> {
    Null, Sym(Sym), Char(Char<'g>),
    Lgl(Vector<'g, Lgl>), Int(Vector<'g, i32>), Real(Vector<'g, f64>),
    Cplx(Vector<'g, Rcomplex>), Raw(Vector<'g, u8>), Str(StrVec<'g>),
    List(ListVec<'g>), Expr(ListVec<'g>),
    Pairlist(Cons<'g>), Lang(Cons<'g>), Dots(Cons<'g>),
    Closure(Clo<'g>), Env(EnvRef<'g>), Promise(Prom<'g>),
    Builtin(Prim), Special(Prim),
    ExtPtr(ExtPtr<'g>), WeakRef(WeakRef<'g>), Bytecode(Ref<'g>), S4(Ref<'g>),
}
```

  `View` has the same variants as today's `Value` (`value.rs:885`) under the
  same names where they exist, so a match is converted by renaming its
  arms (§9.4). `Dots` is a DOTSXP pairlist, as in GNU R.
- **Reads** never need the token: `x.len()`, `x.attrib()`,
  `x.get_attr(sym)`, `cons.car()`, `cons.cdr()`, `cons.tag()`,
  `env.enclos()`, `clo.formals()/body()/env()`, `prom.value()`. Each
  returns a `Ref<'g>` of the same brand. They are `#[inline(always)]`
  loads at core.md §2's offsets.
- **Allocation** takes `&'g Cx` and returns branded results:
  `cx.alloc_real(n) -> FreshVec<'g, f64>`, `cx.cons(car, cdr) -> Ref<'g>`,
  `cx.mk_char(bytes, enc) -> Char<'g>`. Allocation can collect, but it
  runs no R code (finalizers run at safe points, core.md §3). So
  `Ref<'g>`s survive it, and the conservative scan of the current Rust
  segment (§2) keeps them alive.
- **Anything that may run R or C code** takes `&mut Cx`: `cx.eval`,
  `cx.apply`, `cx.force`, `cx.call_native`, `cx.signal_warning_now`, active
  binding reads and writes, package-ALTREP methods, `cx.unique_mut`
  (§1.4). A `&mut Cx` borrow ends every `'g`, so no `Ref` can be used
  across it unless it was rooted.
- **Returning objects from code that ran R code**: such a function writes
  its result into an `Out<'s>` (the VM's destination register, or a
  `RootScope` slot) and returns `RResult<()>`. Safe code has no unbranded
  pointer type ("Fresh"), which is what makes the scheme checkable.
  A builtin's signature becomes

```rust
pub type BuiltinFn = for<'a> fn(cx: &mut Cx, call: Root<'a>, args: Args<'a>, env: EnvRoot<'a>, out: Out<'a>) -> RResult<()>;
```

  `Args<'a>` is a view of the caller's argument window in the VM stack
  (§7.1), rooted by the caller for the whole call and immutable during
  it. `args.get(i).bind(&cx)` gives a `Ref<'_>`. There is no `Vec<Arg>`
  per builtin call, the allocation that `do_bar(..., args: Vec<Arg>, ...)`
  pays today (AGENTS.md).
- **A `Ref` stored in a Rust heap container** (`Vec<Ref<'g>>`) is the one
  hole: the stack scan does not see it (core.md §3, rooting rule).
  Plugging it:
  1. `cx.ref_vec()` returns a `RefVec<'g>`, which keeps its first 16
     elements inline on the stack, where the scan sees them, and spills to
     the root stack (precise) after that;
  2. `tests/no_heap_refs.rs` reads the crate's sources and fails on
     `Vec<Ref`, `SmallVec<[Ref`, `HashMap<…Ref`, `Box<…Ref` and
     `Option<Box<Ref`, like `tests/no_inner_jumps.rs` does for longjmp;
  3. a gctorture conformance job (exists for core/capi, core.md §3).

##### 1.3 Vector views and borrowed reads

```rust
impl<'g, T: Elt> Vector<'g, T> {
    pub fn len(self) -> usize;
    /// Contiguous data, if the vector has it without running code:
    /// inline payload (+32), EXTDATA, robigo's own materialized ALTREP.
    pub fn as_slice(self) -> Option<&'g [T]>;
    /// GNU R's ITERATE_BY_REGION over robigo's own ALTREP classes
    /// (compact seqs, wrappers): chunks of at most 512 elements, never
    /// materializing.
    pub fn regions(self, buf: &mut [T; 512]) -> Regions<'g, '_, T>;
    pub fn elt(self, i: usize) -> T;         // INTEGER_ELT & co.; own classes only
}
/// For package ALTREP classes, whose methods may run R or C code:
impl Cx { pub fn read_regions<T: Elt>(&mut self, x: Root, f: impl FnMut(&[T]) -> RResult<()>) -> RResult<()>; }
```

Soundness rules for `&'g [T]`:

- **Against GC**: the slice's base is an interior pointer into an object
  that the conservative scan of the current segment keeps alive (core.md
  §3 resolves interior and one-past-the-end words). The object does not
  move. A slice cannot outlive `'g`.
- **Against C writes through `REAL(x)`**: C runs only under `&mut Cx`
  (a native call, a callback, a package ALTREP method), and that ends
  `'g`. A C pointer saved across calls (data.table keeps them) can write
  only while C runs, so no Rust slice is alive then. Rust's aliasing
  model is satisfied: no `&[T]` exists while the memory changes.
- **Against Rust writes**: an existing object can be written only through
  `cx.unique_mut(..)` (`&mut Cx`, so every `&[T]` is dead) or through
  a `FreshVec` (unique by construction, `!Copy`). For a kernel that
  reads `y` while writing `x` in place (`x + y` reusing an unshared `x`,
  the ledger's "reusing unshared operands"):

```rust
cx.unique_mut2(x, y, |xs: &mut [f64], ys: &[f64]| ..)?   // checks x != y (one compare), else copies
```

- **Against ALTREP materialization**: `as_slice` returns `None` for a
  package class with no data (it never calls `Dataptr` from `&Cx`
  code), and builtins take `read_regions`. GNU R calls the class's
  methods exactly where it calls them. The bridge's eager copies and the
  "deferred" forcing list (core.md §6, "Deferred package ALTREP
  strings and lists", the `may_stay_deferred` whitelist) disappear.
- `FreshVec<'g, T>`: `as_mut_slice(&mut self) -> &mut [T]`, then
  `finish(self) -> Ref<'g>`. Allocation leaves numeric payloads
  uninitialized, as core.md §3 says. `FreshVec` exposes
  `&mut [MaybeUninit<T>]` until it is filled, or `alloc_real_zeroed` for
  GNU R's `allocVector` + `memset` sites.

##### 1.4 Copy-on-modify

- `refcnt` (+4, u16, saturating) is GNU R's `REFCNT`. The core already
  maintains it in `Sexp::store` (`lib.rs:605`, `FIX_REFCNT`). Bindings,
  list elements, attributes, cons cars and cdrs, and `PRVALUE` count.
  Locals, VM registers, `Args` windows and the protect stack do not.
- `MAYBE_SHARED(x)` (`refcnt > 1`) decides in place against copy, at the
  same places as GNU R's C (`do_subassign`, `do_subassign2`,
  `applydefine`'s `evalseq`, `SETCAR` in `do_attrgets`, …).
  `cx.unique_mut(x)` is `if MAYBE_SHARED(x) { shallow_duplicate }`.
- **VM stack values**: port GNU R's deferred link counting
  (`eval.c:991-1050`). The VM keeps `prot_committed`; before an op that
  can mutate (`SetIndex*`, `SetVar` of an object, a replacement
  function call: GNU R's `STARTASSIGN`/`INCLNK` points),
  `INCLNK_stack_commit` increments the counts of the object slots below
  the protect top, and `DECLNK_stack` undoes it when the window pops. The
  functional-style code pays nothing. `c(x, {x[1] <- 0; x})` copies,
  as GNU R's byte code does.
- **The "take by value" rule (porting contract §5) is retired.**
  Replacement functions receive `x` in the argument window. Refcounts
  say whether they may write. `x[i] <- v` in a loop writes in place,
  because the binding's reference is the only one. The `RESTORE`
  thread-local in `subassign.rs:41` and the `own_var`/`set_var` tricks
  (perf-vm, "redo, don't port") go away.
- Growth (`x[n+1] <- v`): GNU R's growable vectors, ported exactly:
  `GROWABLE` gp bit, `TRUELENGTH` = capacity, `expand = 1.05`
  (`subassign.c:177`), because `TRUELENGTH` is visible to C (data.table)
  and to `.Internal(inspect)` (`tl=`). An optional, invisible
  optimization: when the new capacity still fits the slot's size class
  (allocations are carved at class size, core.md §3), extend in place and
  set `length`/`truelength` to GNU R's values without copying. Only
  `.Internal(address())` could tell, and it differs from GNU R's anyway.

##### 1.5 Strings in the API (detail in §5)

`Char<'g>` is a CHARSXP `Ref`: `bytes() -> &'g [u8]` (CHARSXPs are
immutable), `enc() -> Enc`, `is_ascii()`, `is_na()`, `hash() -> u32` (+8),
`utf8(&self, cx: &Cx) -> Cow<'g, str>` (zero-copy for ASCII, UTF-8 and
native in a UTF-8 locale; translates otherwise, with sysutils.c's escapes).
`StrVec<'g>::as_slice() -> &'g [Char<'g>]` (pointer-sized elements).

---

#### 2. Rooting Rust frames

##### 2.1 What other systems do

| system | Rust/C++/C frames | files (from memory; paths may have moved) |
|---|---|---|
| GNU R | precise `PROTECT` stack; `rchk` (LLVM static checker) finds misses | `src/main/memory.c` (`R_PPStack`), rchk by T. Kalibera |
| V8 | precise `HandleScope`/`Local<T>` (indirect, objects move); conservative stack scanning added for Oilpan and as `--conservative-stack-scanning` | `src/handles/handles.h`, `src/heap/conservative-stack-visitor.cc` |
| rusty_v8 (Deno) | `HandleScope<'s>` / `Local<'s, T>` lifetime-branded handles | `src/scope.rs`, `src/handle.rs` |
| SpiderMonkey | `Rooted<T>` RAII and a static hazard analysis that runs in CI | `js/public/RootingAPI.h`, `js/src/devtools/rootAnalysis/` |
| JavaScriptCore | conservative scan of the machine stack and registers; precise heap | `heap/MachineStackMarker.cpp`, `heap/ConservativeRoots.cpp` |
| Julia | precise GC frames (`JL_GC_PUSHn`) in C; compiled code's roots and derived pointers placed by an LLVM pass | `src/julia.h`, `src/llvm-late-gc-lowering.cpp` |
| Ruby | conservative machine stack plus `RB_GC_GUARD` | `gc.c` (`mark_current_machine_context`) |
| Lua | values live only on the Lua stack; C code sees stack indices | `lapi.c`, `lstate.h` |
| gc-arena / piccolo | branded `Gc<'gc>`; collection only *between* mutations, with a stackless interpreter | `gc-arena/src/arena.rs` |

Two lessons from robigo's own ledger decide between them.

- Conservative scanning of the evaluator's frames breaks **finalizer
  determinism**, which conformance needs: stale words kept dead objects
  alive "at random, so a finalizer ran one `gc()` late (R6's finalizer
  tests under rlang's `eval_bare`)" (core.md §3; ledger 2026-10-28). The
  2026-11-11 entry found a stale pointer in a `Value`'s seven padding
  bytes, and another in a `jmp_buf`'s unwritten signal mask.
- Fully precise rooting of 60k lines of builtins is GNU R's `PROTECT` tax,
  whose bugs (unprotected values across allocation) are rchk's whole
  reason to exist. Rust can check rooting at compile time, but only
  across `&mut` borrows. Making allocation take `&mut` as well conflicts
  with passing branded arguments to the allocator (`cx.cons(x.car(), y)`
  does not borrow-check). That forces rusty_v8-style "every read pushes
  a handle", with nested scopes in every loop.

##### 2.2 The decision: precise across callbacks, conservative within a Rust stretch

A collection marks:

1. the precise roots: the protect stack (also Rust's `RootScope`s, §2.3),
   the precious set, the API globals, the `Cx` global table, the context
   stack, the handler and restart stacks, the VM stack (`[0, top)`, tags
   checked, §7.1), the symbol table's per-isolate cells (§3.6) and the
   pending-condition slot (§4.5);
2. **C segments** conservatively, exactly as today (C locals that GNU R
   tolerates by luck; `robigo_try_call65`'s narrowed boundary and 1 KiB
   clear, ledger 2026-11-11);
3. **the current Rust segment** conservatively: the frames from the
   most recent *R entry* down to the collector, after spilling the
   callee-saved registers. Any earlier Rust segment is suspended and not
   scanned;
4. weak words (§7.3) are cleared, not marked.

An **R entry** is one of these, the only functions through which Rust
runs R code: `Cx::apply_closure`, `Cx::eval` (AST), `Cx::force`, the VM's
entry, `Cx::call_native` (which then opens a C segment), the finalizer
runner, and `do_gc`. Each one saves the current segment's top, sets it to
the caller's stack pointer (the address of a marker local, as
`ScanSegment` does today, `gc.rs:332`) and restores it on return or
`Err`. That costs about 4 instructions per closure call. Leaf builtins
called from the VM do not open a segment: they run in the VM's.

Why this is sound:
- Safe code: a `Ref` held across an R entry would need a `&'g Cx` borrow
  alive across a `&mut Cx` call, which the borrow checker rejects. So
  suspended frames hold no live unrooted `Ref`s. Within a segment,
  `Ref`s live across allocations are on the scanned stack, and interior
  pointers resolve (core.md §3).
- The evaluator core (`rt/eval`, `rt/vm`, raw `Sexp`, `unsafe`) keeps
  this invariant by construction: **every object live across an R entry
  is in a VM stack slot, a frame, a context or a root.** Lua and CPython
  keep the same invariant (values on the VM stack). In the core's
  segment, anything else it holds across allocation is scanned as a
  safety net.

Why it is deterministic where GNU R is: an explicit `gc()` reaches
`do_gc` through an R entry. Its segment holds no objects (it calls
`collect` directly; its body is ~10 lines), so the stack is scanned only
in C segments, which is today's behaviour, measured deterministic by
`weak-keys-collections.R` and the 2026-11-11 stress script. Stale words in
suspended frames, which caused both ledger bugs, are never read.
Collections triggered by allocation may still see stale words in the
current segment, so a finalizer can run one collection later. GNU R's
allocation-triggered timing is not reproduced bit for bit anyway: object
sizes differ, and `heapsize.rs` only approximates it. No conformance case
can rely on it.

##### 2.3 Precise roots in Rust: `RootScope`

```rust
let s = cx.scope();                       // records the protect stack top
let acc = s.root(cx.alloc_list(n).finish());   // R_PPStack[top++] = p; returns Root<'s>
for i in 0..n {
    cx.apply(f, &[args.get(0), acc.into()], s.out())?;    // R code: `acc` is a Root, so it survives
    ...
}
// drop(s): protect stack top restored (LIFO by scope nesting)
```

Rust roots share the C protect stack (`R_PPStack`, per isolate). A root
costs one store and one increment. `RootScope` truncates on drop, and its
handles are `Copy` indices, so roots never pop out of order (rusty_v8's
model). The stack overflow error is GNU R's ("protect(): protection stack
overflow"). `R_PPStackSize` defaults to GNU R's 50,000. Contexts save and
restore `R_PPStackTop` (core.md §5), so a `Flow` that unwinds through
Rust restores it.

##### 2.4 VM register files, JIT frames, LLVM

- **VM**: slots in the isolate's VM stack are precise roots (§7.1). The
  dispatch loop runs inside the segment of its R entry, so its spill
  slots are a safety net for allocation-triggered collections. They do
  not count at R entries: every call op has its operands in slots.
- **Cranelift code**: at every helper call that can run R code, the
  generated code writes its live object values back into the VM slots.
  It already does so for deopt (architecture.md "Loops deoptimize to the
  VM at op granularity"). Native frames are inside the VM's segment, so
  allocation-triggered collections scan them conservatively. Cranelift's
  stack maps (user stack maps) are not needed.
- **LLVM tier (G8)**: the same contract, so no `gc.statepoint`. LLVM may
  keep only derived pointers. Interior pointers and one past the end
  resolve (core.md §3). A derived pointer *below* an object's start, as
  strength reduction of 1-based loops makes, lands inside the 32-byte
  header for every offset of up to 4 elements, so it resolves too. Larger
  negative offsets are theoretically possible (Boehm's "GC-safe
  compilation" caveat) and are only a hazard in the current segment
  between allocations. Measurement: §11, R5.
- Callee-saved registers are spilled when the scan starts, and
  caller-saved ones are spilled by the compiler into the frames being
  scanned. XMM registers are caller-saved on SysV, so a pointer the
  compiler parked there across the call is in a scanned frame.

##### 2.5 What changes in robigo-core

- `ScanSegment` gains a kind: C segments are kept closed as today, and
  Rust segments are "current or suspended". `Core::scan_top` becomes the
  current Rust segment's top. `collect_explicit()` (for `do_gc`, `R_gc`)
  scans only C segments.
- After G3, nothing uses holds, watched objects, twins, the proxy host
  trace, `HOST`, `HOST_OWNED`, `TWIN` or `WATCHED`. They are deleted
  (gc.rs:976-1027, lib.rs:57-71), which removes the "one mark over both
  heaps" machinery of core.md §6 (#344).

---

#### 3. Environments

##### 3.1 Layout

An ENVSXP is a node whose only fixed field is ENCLOS at +24 (council item
20, core.md §2), so the frame words are free to redesign. Proposal: a
**variable-size environment with inline slots**, so that a call frame is
one allocation, as it is today ("environments keep up to four bindings
inline (one allocation)", design.md §4.2):

```text
ENVSXP (size class chosen at allocation: 48 + 16 + cap*9 rounded to 16)
+0   header        type 4; gp: FRAME_LOCK (1<<14), GLOBAL_FRAME (1<<15) as GNU R;
                   robigo: SEARCH (env is on the search path), BASE (frame = symbol table)
+8   attrib
+16  frame         → the slot block: self+48 when inline, or a FRAMESXP after growth
+24  ENCLOS        (fixed; OFF_ENCLOS)
+32  index         R_NilValue, or a RAWSXP open-addressing index sym → slot (cap > 8)
+40  shape         R_NilValue, or the CODESXP whose slot layout this frame was made from
+48  inline slot block (when present):
     +0  n: u32 (slots used) · cap: u32
     +8  fn_version: u32 · order_next: u32
     +16 mask: u64                         (the 64-bit symbol filter, env.rs:76 today)
     +24 syms[cap]: Sexp                   (symbols; immortal, not traced)
     +24+8cap vals[cap]: u64               (Sexp, or unboxed scalar bits)
     +24+16cap flags[cap]: u8              ACTIVE 0x01 · LOCKED 0x02 · MISSING 0x0C (2 bits)
                                           · MISSING_DEFAULT 0x10 · UNBOXED 0x60 (none/real/int/lgl)
                                           · ARG_FORCED 0x80 (§4.2)
     order[cap]: u16                       (creation order, for ls(sorted=FALSE)/as.list)
```

`FRAMESXP` is a robigo-private type 26 with the same slot block after a
vector header. GNU R uses 0-25, and 30 and 31 for its own NEWSXP and
FREESXP (`Rinternals.h:141`), so 26-29 are free. C never sees it:
`FRAME()`/`HASHTAB()` are already shims that materialize GNU R's pairlist
or hash table (core.md §2, `robigo_core::env`). The collector traces
`vals[i]` whose `UNBOXED` bits are clear, exactly like GNU R's
`BNDCELL_TAG` (`Defn.h:462`). This is a small core change: a trace
function for type 26 and for ENVSXP with an inline block.

- **Function frames**: the compiler lists formals and the local `<-`
  targets of the body. The frame is allocated with those symbols
  prefilled, their values `R_UnboundValue`, and `shape` = the closure's
  CODESXP. The VM addresses a variable by slot index after checking
  `env.shape == code` once per frame (it is the frame the VM itself
  made, so the check folds away for locals). `assign("z", …)`, `eval`
  of arbitrary code and C's `defineVar` append to free slots, or grow
  the frame into a FRAMESXP. Indices stay stable, so cached slot hints
  stay valid.
- **Removal** (`rm`) sets `R_UnboundValue` and keeps the slot, as
  `Frame`'s `VACANT` does today (`env.rs:66`); compaction happens when
  half the slots are vacant, which bumps `fn_version`.
- **Order**: GNU R lists unhashed frames in reverse creation order and
  hashed ones by bucket. `order[]` is set when a slot is first bound,
  which reproduces both through the existing `nbase` logic.
- **Global env, namespaces, package envs, `new.env(hash = TRUE)`**: the
  same block with an index (`+32`). GNU R's `HASHTAB` order for
  `ls(sorted = FALSE)` is computed by the shim from GNU R's hash
  function (`R_Newhashpjw`), so the order matches without storing GNU
  R's buckets.
- **Base env and base namespace**: their bindings live in the
  per-isolate symbol table (§3.6), as GNU R keeps them in `SYMVALUE`.
  The two environments share it (today's `FrameCell::Shared`,
  `env.rs:406`).

##### 3.2 Binding operations, flags, locking

One module (`rt/env.rs`) is the only reader of raw slots. It exports
`get(&Cx, env, sym) -> Lookup<'g>` (Value, Unbound, Active, Promise,
Missing, …), `define(&mut Cx, …)`, `set_super`, `remove`, `lock`,
`make_active`, `iter_ordered`. The `&mut` ones may call active-binding
functions. Every write goes through `Sexp::store` (barrier and
FIX_REFCNT, `lib.rs:605`). GNU R's errors are verbatim:
"cannot change value of locked binding for '%s'", "cannot add bindings
to a locked environment".

##### 3.3 Version counters for VM guards

- `fn_version` (in the slot block) is bumped when a binding that holds
  or held a function, or an active binding, is created, changed or
  removed. This is today's `EnvData::fn_version` (`env.rs:423`).
- `search_epoch` (in `Cx`) is bumped on attach, detach and the
  namespace import table.
- The guard memo key stays (search epoch, environment address,
  `fn_version`), checked once per closure environment (perf-vm "For G3").
  Environment ids (`NEXT_ENV_ID`, `env.rs:449`) become addresses plus a
  per-allocation generation, or the memo cell lives in the env's
  shape/CODESXP, so that a reused address cannot validate a stale memo.
  Which of the two is cheaper is prototype decision D2c.

##### 3.4 `<<-`

GNU R's `setVar`: walk from `ENCLOS(rho)`, assigning in the first frame
that binds the symbol, else in the global env. VM call-site cache:
(depth `d`, the expected `shape` of the target frame, slot `k`). On
execution, walk `d` enclosures (usually 1), check `shape == cached`,
check that the frames skipped do not bind the symbol (their shapes are
fixed and do not list it; a dynamic append bumps a per-frame "dynamic
bindings" count that the check also reads), check flags (not ACTIVE,
not LOCKED), then store. This is about 8 loads for `bal <<- bal + x` in
`calls.R`, where robigo is 1.39x GNU R today.

##### 3.5 C API

`Rf_findVarInFrame3`, `Rf_defineVar`, `R_existsVarInFrame`,
`R_lsInternal3`, `R_MakeActiveBinding`, `R_LockBinding` and friends call
`rt/env.rs` directly. There is no host callback and no proxy (core.md
§6's HOST environments go). `R_GetVarLocValue`/`R_findVarLocInFrame`
(methods needs them, core.md §13) return a cell: robigo gives a
`R_varloc_t` that encodes (env, slot), whose accessors go through
`rt/env.rs`. Unboxed values are boxed into their slot on the first read
as an object (GNU R's `R_expand_binding_value`), so C's identity
guarantees hold. This is today's `Env::binding_value` rule (core.md §6),
which carries over unchanged.

##### 3.6 Symbols, base bindings, the global cache

- Symbols live in an immortal symbol segment reserved as one virtual
  range per process (core.md §10: shared, interned under a lock).
  `Sym::id() = (addr - base) / 48` is dense with no header field, and
  `sym_bit` (`id & 63`) keeps today's 64-bit masks.
- The per-isolate symbol table is `Cx.syms: Vec<SymCell>`, indexed by
  id, 16 bytes per symbol: `base_value: Sexp` (base env bindings,
  GNU R's SYMVALUE), `global: GlobalCacheEntry` (GNU R's
  `R_GlobalCache` entry, `envir.c:616-683`: the binding found on the
  global/search path for this symbol, validated by `search_epoch`).
  Looking up a function from the global env is one indexed load and one
  compare. `SYMVALUE()` for base natives (one use, core.md §1) is a shim
  over `syms[id].base_value`.
- This is what makes isolates work. Symbols are shared and immutable;
  values are per isolate (core.md §10 says "a symbol's value cell, used
  only by base, moves into the isolate").

---

#### 4. Closures, promises, argument matching, contexts

##### 4.1 Layouts (48-byte nodes, core.md §2)

| type | +16 | +24 | +32 | +40 (4th word) |
|---|---|---|---|---|
| CLOSXP | FORMALS | BODY (the source, never replaced by bytecode; `body(f)` and C see it) | CLOENV | CODESXP: compiled code, inline info, default-argument code, JIT entry (today's `Closure.code/inline/defaults/jit` OnceCells, `value.rs:614`) |
| PROMSXP | PRVALUE (`R_UnboundValue` if unforced) | PRCODE (the expression) | PRENV (`R_NilValue` once forced, as GNU R) | before forcing: CODESXP of the expression (VM promise code); after forcing with gp `UNBOXED`: the scalar's bits (tracer checks gp) |
| LANGSXP / LISTSXP / DOTSXP | CAR | CDR | TAG | call-site cache cell (weak, §7.3), or R_NilValue |

`CODESXP` (private type 27) is a vector-header object:
`+16 Box<Code>` (released by the sweeper's release callback; the core
already has `ReleaseFn`), `+24` constant pool VECSXP (traced), `+32`
frame shape (symbols per slot), `+40` the weak cache-word block. Because
the code cache is an object reachable from the closure, an unused
closure's code dies with it. Today's `vm.rs:219` `CACHE` thread-local,
which never evicts (design.md §5.2), goes.

GNU R's `PRSEEN` is gp bits 0-1 of PROMSXP; `Promise::interrupted`
(perf-vm 2026-11-10) is `PRSEEN == 2`, set by `R_restore_globals`.

##### 4.2 Calls

A general closure call in the new VM (`f(i * 0.5)` with two defaults):

1. Look the function up through the call-site cache (weak cell in the
   LANGSXP's 4th word, validated by guard memo).
2. Match arguments. A static call site keeps a memoized permutation per
   (site, callee formals), as today (`no_eager`, permutation memos). The
   general path ports `matchArgs` (3 passes) onto the argument window.
3. Allocate the frame: **one** object (§3.1), slots prefilled from the
   CODESXP shape.
4. Argument values:
   - *Strict formals with leaf/simple arguments* (today's `Code::strict`
     plus `caller_promargs`, perf-vm 2026-10-28): the caller computes the
     value and stores it **in the slot, flagged `ARG_FORCED`**, without
     allocating a promise. When something asks for the promise
     (`substitute`, `missing`, `sys.function`/`match.call` through
     `promargs`, C's `findVarInFrame`, `as.list(env)` or `get` when GNU R
     would see a PROMSXP), `rt/env.rs` materializes
     `PROMSXP{PRVALUE = value, PRCODE = the call's i-th argument
     expression, PRENV = nil}` into the slot. GNU R's `promiseArgs` uses
     `CAR(el)` of the call as PRCODE, so the identity is the same. This
     generalizes today's `BIND_MISSING_DEFAULT` trick (core.md §6,
     "Constant formal defaults") from constants to every caller-forced
     argument. Decision D4b measures it against a real forced promise.
   - Other arguments get a PROMSXP (one 48-byte bump allocation, whose
     aux is the argument's CODESXP).
   - Constant defaults are bound as constants, flagged `MISSING_DEFAULT`
     (as today). Non-constant defaults get a promise with code.
5. Push a context (§4.4) with **no allocation**: `promargs` is
   materialized lazily from (argument window, names) only when
   `sys.function`, `match.call` or `sys.call` with `expand.dots` ask
   for it.
6. Open the Rust segment (4 instructions), run the body in the VM, pop.

Allocations per call: 1, plus one per lazy argument. Today: frame,
promises, argument vector and boxed results (perf-vm "Next"). GNU R: env,
one cons per binding, one promise per argument, the promargs pairlist.

##### 4.3 `...`

DOTSXP pairlist of promises (or values flagged ARG_FORCED in the cons'
gp), bound to `...` as in GNU R. `..1`, `...length()`, `...elt()` and
`list(...)` are VM ops over the cons list. Dots forwarding (`f(...)`)
splices without copying the promises, as GNU R does.

##### 4.4 The context stack (RCNTXT for C)

`Cx.ctx: Vec<Ctx>` (heap memory, a precise root):

```rust
###[repr(C)]
pub struct Ctx {            // 96 bytes; field set from Defn.h:1319 RCNTXT
    callflag: u32, evaldepth: u32,
    call: Sexp, callfun: Sexp, cloenv: Sexp, sysparent: Sexp,
    promargs: PromArgs,     // lazy: (vm window base u32, n u16, names) or a materialized Sexp
    conexit: Sexp,          // on.exit expressions (R_NilValue when none)
    srcref: Sexp,
    pp_top: u32, vm_top: u32, handler_top: u32, restart_top: u32,
    vmax: u32, flags: u16 /* intsusp, gcenabled, browserfinish, bcintactive */, _pad: u16,
}
```

- `sys.call`, `sys.function`, `parent.frame`, `sys.frames`,
  `on.exit(add, after)` and `returnValue` read it. They are ports of
  context.c.
- The handler and restart stacks are `Vec`s of `#[repr(C)]` entries with
  `Sexp` fields (precise roots). GNU R's `R_HandlerStack` is a pairlist,
  materialized only for C callers that read it (`R_GetHandlers`, none in
  the census as far as I know: a guess to check against
  `tests/capi/symbols.tsv`).
- C's RCNTXT: base-package sources that `begincontext` (they are built
  against robigo's `Defn.h`, core.md §8) get robigo's `RCNTXT` layout.
  `begincontext`/`endcontext` exports push and pop `Cx.ctx` and the C
  `jmp_buf` context (`shim.c`), as `errors::Ctx` does now (core.md §12,
  item 3).

##### 4.5 `Flow` and unwinding

- `type RResult<T> = Result<T, Flow>`, where `Flow` is **8 bytes**:
  `{ kind: u8, target: u32 }` (error, return to ctx N, break, next,
  restart, interrupt, exec-continuation). The payload (the condition
  object, the returned value) is in `Cx.pending: Sexp`, a precise root,
  like GNU R's `R_ReturnedValue`/`R_RestartToken`. `Result<(), Flow>`
  fits in one register, which today's `Flow` does not (it carries
  `Value`s). Unwinding frames run `on.exit` code (R code, `&mut Cx`)
  with the payload rooted.
- Errors raised from `&Cx` code (allocation failure, a type error in a
  kernel) are created as `Flow` plus pending, and calling handlers run
  at the first R-entry boundary the `Err` reaches. That boundary is the
  builtin's caller, before any R code runs, so the order is the same as
  GNU R's `errorcall` (handlers run before unwinding).
- **Warnings from `&Cx` code** are queued (`cx.warn_deferred(call,
  msg)`) and signalled, in order, when the builtin returns, *before* its
  result is visible to R code. For a builtin without side effects this
  cannot be told apart from GNU R's immediate `warningcall`: no R code
  runs in between, and with `warn = 2` or a restart that aborts, the
  result is discarded as in GNU R. A builtin *with* side effects (writes
  to connections, `assign`) uses `cx.warning_now(..)` (`&mut`). The
  rule is tested by a source scanner: a builtin marked `pure` may use
  only deferred warnings, and every other builtin uses `warning_now`
  unless it is listed with a reason. This is a risk, ranked in §11 (R6).
- C ↔ Rust ↔ C: unchanged from core.md §5 (trampolines, `Res`, only
  export wrappers jump). The **conversion layer disappears**: `Rf_eval`
  from C calls `Cx::eval` on the same objects and returns `Result`;
  `.Call` passes argument `Sexp`s as they are. The twin, hold, proxy and
  ALTREP-copy machinery of core.md §6 is deleted.

---

#### 5. Strings

- `RString`/`RStr` (`value.rs:43-93`, 286 uses) become `Char<'g>`
  (CHARSXP) and `StrVec<'g>`. `NA_character_` is the immortal
  `NA_STRING` (core.md §4). Encodings are the gp bits, and their rules are
  already in `robigo_core::chars` (core.md §4, "so G3's runtime uses the
  same ones"); the runtime needs no `Encoding` enum of its own.
- **What builtins see**: `bytes()` everywhere bytes are meant
  (`nchar(type = "bytes")`, `charToRaw`, `useBytes = TRUE`, hashing);
  `utf8(&cx)` where GNU R calls `translateCharUTF8`; `native(&cx)` where it
  calls `translateChar`. Every `translateChar` call site in GNU R's C
  maps to one of these, which makes the encoding behaviour a 1:1 port. The
  mapping of `RString`'s byte-to-U+00FF trick (core.md §4.1) is gone.
- **Construction**: `cx.mk_char(&[u8], Enc)` is one hash and one probe
  of the flat CHARSXP table (council item 10). `paste` builds each
  element in a reused `Vec<u8>` scratch buffer, then calls `mk_char`,
  as GNU R's `paste` does with `R_alloc`. A small-integer string cache
  (today `coerce.rs:338` `SMALL`) becomes a per-isolate `Vec<Sexp>` of
  CHARSXPs, a `Global` root.
- **`match`/`unique`/`duplicated`/`%in%`/`factor`**: port unique.c's
  string paths, which hash the CHARSXP *pointer* when every string is
  ASCII or has the same known encoding (`useCache`), and translate to
  UTF-8 only otherwise. Equality is a pointer compare. This should beat
  today's `Rc<str>` content hashing (an expectation, measured in §10 by
  `robigo/strings` and `knucleotide`).
- **`TRUELENGTH` of CHARSXPs** stays free package scratch (core.md §4).
  robigo's own algorithms do not use it, so data.table keeps working.
- **Symbols' print names** are CHARSXPs, so `as.character(quote(x))` is
  pointer-equal to `"x"`, which GNU R code relies on.

---

#### 6. Vectors and ALTREP

| storage | how | who | notes |
|---|---|---|---|
| inline | payload at +32 | default | length-1 vectors are 48 bytes |
| EXTDATA | +32 data ptr, +40 owner, +48 release fn | C-owned writable memory | after G3 its only users are foreign writable buffers. `HOST_OWNED` goes |
| robigo ALTREP | class implemented in Rust (`Methods::own`) | compact `intseq`/`realseq`, wrappers, deferred coercion strings | `elt`, `regions`, `sum`, `is_sorted`, `no_na` callable from `&Cx`; `1:2e8` is 48+ bytes (#460) |
| foreign ALTREP | Arrow/NumPy/mmap classes (core.md §2 "Arrow buffers") | interop | `as_slice` read-only through `Dataptr_or_null`; writable access materializes once |
| package ALTREP | `R_make_alt*_class` | cli, vroom, vctrs, arrow | methods called where GNU R calls them, behind `&mut Cx` (`read_regions`, `elt_cx`). No eager copy (#422 and #468 machinery deleted) |

- **VM range registers** (`RVal::Range`, `vm.rs:51`) become compact
  intseq objects allocated once per loop (GNU R does the same). `for (i
  in 1:n)` iterates over the class's region without materializing.
- **Attributes**: the pairlist at +8, as GNU R, read by `get_attr(sym)`
  (linear; attribute lists are short). `OBJECT` and `S4` bits are kept in
  sync by `setAttrib`/`classgets` (core.md §2). The S4 bit exists from
  day one, which unblocks G6 (core.md §13 "a runtime flag … is the first
  G6 change").
- **`tracemem` (#140)**: GNU R's trace bit (`extra` RTRACE on
  non-closures) and the "tracemem[0x… -> 0x…]" report in
  `duplicate`. Because addresses are real now, the report shows them.
- **Kernels** (`arith_exact.rs`, x87 reductions, LSD radix `order`,
  `duplicated` index tables): unchanged algorithms over `&[T]`/`&mut
  [T]`. They are representation-independent, which the ledger confirms.

---

#### 7. VM and JIT on `Sexp`

##### 7.1 Register file

- The **isolate VM stack**: `vals: *mut u64` and `tags: *mut u8`,
  structure of arrays, `cap` words (default 1 Mi slots, grown in chunks
  so pointers into it stay stable; a guess, sized to `R_BCNODESTACKSIZE`
  semantics), `top`. A frame is a window `[base, base + nregs)`. Pushing a
  frame bumps `top`; popping is a store. There is no `SmallVec`, no drop
  glue and no reserve cost (perf-vm's `run_pre` costs, "redo, don't
  port").
- Tags: `OBJ 0` (payload is a `Sexp`), `REAL 1`, `INT 2`, `LGL 3`,
  `EMPTY 4`. There is no NaN-boxing: R's NaN payloads (NA_real_ is
  0x7FF00000000007A2) must survive bit for bit. Today's `RVal` is a
  24-byte enum (`vm.rs:41`). Here a slot is 9 bytes, a tag test is one
  byte load, and the GC scans the tags linearly.
- Unboxed slots are boxed when they escape (`vm::escaping_read`'s rules,
  core.md §6 "Scalar variables", which carry over: a computed scalar
  handed to C is a fresh object, a variable's value is boxed into its
  binding once).

##### 7.2 What carries over (perf-vm "For G3"), and what is redone

Carry over: call-site function lookup through local frames; pure-closure
inlining with leaf and single-arithmetic arguments; caller-forced
promises (now `ARG_FORCED` slots, §4.2); index promises forced in R's
order; guard memoization keyed on (search epoch, env, `fn_version`);
static rejection memos (`no_eager`); `%*%` row-vector dot products; the
measurement method (`BYFN=1 cmp.py`, `linediff.sh`, `asm.py`).

Redo: register buffers (now §7.1); `set_var`/`own_var` (now refcounts
and slot frames); the cold-path layout of `run_with` (`vm.rs:5922`):
keep the dispatch loop small, measured per instruction against the base.

##### 7.3 Inline caches are weak

A call-site cache that holds a closure strongly keeps the closure's
environment alive. With R6, `obj$method()` would then keep `obj` alive,
and its finalizer would not run at `gc()`. GNU R has no such cache, so
this would be a conformance bug. Caches therefore store **weak words**:
the CODESXP's cache block (`+40`) is registered with the collector, which
zeroes every word whose target is unmarked after marking, before
sweeping. The CHARSXP cache is already pruned the same way (core.md §3).
JSC clears weak inline-cache references in `finalizeUnconditionally`,
and V8 has weak slots in feedback vectors. The memos that hold
`Weak<Closure>` today (`interp.rs:77`, `vm.rs:3804`) become weak words.

##### 7.4 JIT ABI against header offsets

```rust
type LoopFn = unsafe extern "C" fn(cx: *mut Cx, vals: *mut u64, tags: *mut u8, base: u32) -> u32;
```

The function reads and writes VM slots directly at `vals + 8*slot` and
`tags + slot`. Today it gets a `VecView` copy and `LoopCtx`
(`jit.rs:134`). Deopt writes the live state into the slots, so
"write back and let the VM execute that op" (architecture.md) is a
handful of stores. Vector access inline:

```text
t  = load.u8 [x + 0]                 ; TYPEOF
f  = load.u8 [x + 1]; test f, ALTREP|EXTDATA → slow helper
n  = load.i64 [x + 16]               ; XLENGTH
p  = x + 32                          ; DATAPTR
REAL/INT store: plain store; STR/VEC store: test BARRIER (f & 0x10) → helper, else FIX_REFCNT inline (u16 at +4) and store
```

The offsets come from `robigo_core` constants (`OFF_LEN`, `OFF_DATA`,
`lib.rs:165-181`), so a layout change is a compile error in the JIT, not
silent corruption. JIT frames are rooted as §2.4 says (spill to slots at
R-running helpers; conservative safety net in the VM's segment).

##### 7.5 Compile-time binding of base functions

perf-vm 2026-10-28 records that GNU R's JIT binds base functions at
compile time, while robigo re-checks guards. G3 does not change that; it
stays a conformance decision for the VM track.

---

#### 8. The isolate record

`Cx` holds everything per isolate: the heap (`Core`, embedded so the
allocation fast path is two loads off `cx`), the symbol cells (§3.6),
`global`/`base`/`base_ns`/`empty` (as `Global`s), the context, handler
and restart stacks, the VM stack, the root/protect stack (shared with
`R_PPStack`), `pending`, options, connections, RNG state, warnings,
`R_Visible`, `R_Srcref`, eval depth, `numerics`, epochs, the JIT module,
the console, and **all 69 `thread_local!`s** of robigo-runtime (listed by
`rg 'thread_local!' crates/robigo-runtime/src`: the symbol interner
`sym.rs:153`, `vm.rs:219` CACHE and INLINES, `env.rs:449` NEXT_ENV_ID,
`altrep.rs:45` REGISTRY, `value.rs:495-496` compact registries,
`jit.rs:598` MODULE, `format.rs:407` OUT_DEC, `console.rs:142`, …). Most
of them disappear (registries, caches, the interner) because objects
carry their own data.

Access:

- **Rust**: `&mut Cx` (or `&Cx`) passed down, as `&mut Interp` is today.
  There is no TLS read on any Rust hot path.
- **JIT code**: `cx` is an argument (§7.4), kept in a callee-saved
  register.
- **C API exports**: one load of the current isolate. Proposal: a plain
  process-global `ROBIGO_SOLE: *mut Isolate` that is non-null while the
  process has exactly one isolate (the normal case). Exports test it
  first and fall back to TLS when several isolates exist, which is a
  well-predicted branch. Per-isolate API data (`R_GlobalEnv`,
  `R_PPStackTop`, `R_Visible`, …) are macros over that accessor in
  robigo's headers (core.md §10).
- **libR.so revisit (item 15)**: after G3 the runtime's TLS traffic
  (535,920 `__tls_get_addr` calls at startup, ledger 2026-11-10) is
  gone, and exports read the sole-isolate global. What is left is the
  out-of-line allocator shim. The rerun is one command pair (§10, M8),
  and it becomes a cheap decision.
- **capi's per-isolate state** (`state::ISOLATE` today) merges into
  `Cx`: robigo-capi's functions take `&mut Cx` from the export wrapper,
  and its `host.rs` callback table goes (the runtime *is* the host).

---

#### 9. Migration strategy

##### 9.1 Options

| | plan | verdict |
|---|---|---|
| A | coordinator: narrow `Value` on main, then one swap on a long-lived integration branch | **no** |
| A' | a target-API facade over `Rc` on main, then swap the implementation | second best |
| B | **a new crate `robigo-rt` on main, grown beside `robigo-runtime`, with its own ratchets; flip, then delete** | **recommended** |
| C | a staged mixed Rc/GC heap in one runtime | no (agree with the coordinator) |

**Against A.** Step 3 (private variants behind constructors and a
typed `view()`) cannot express the target. An `Rc`-backed `view()` has
no lifetime tied to a GC token, no `&Cx`/`&mut Cx` split, no
uniqueness-by-refcount, no cons cells and no CHARSXPs. So the ~4,900
match sites, 142 `Rc::make_mut` and 286 `RString` uses would be edited
twice: once to the narrowed API, and again when the lifetimes and the
`&mut` discipline arrive. The second edit is the large one, because it
changes control flow (rooting across callbacks, warnings, in-place
decisions), not spelling. The swap itself is a big bang. The integration
branch compiles nothing until value.rs, env.rs, interp.rs, vm.rs,
compile.rs and jit.rs are all redone (~20k lines), and it sees no
ratchet signal until base boots. Meanwhile main moves on, and every
rebase conflicts with exactly the files being rewritten. "Swap local to
`value.rs`" is not achievable: the semantics move (Rc drop versus
tracing, `Rc::make_mut` versus REFCNT, `Vec<Arg>` versus cons cells,
`RefCell` frames versus slot frames).

**Against C (the "no mixed heap" argument, evaluated).** The coordinator
is right, and the core ledger is the evidence. Attributes, list
elements, promise values and environments can point anywhere, so every
field would need to be either kind. Cross-heap cycles needed "one mark
over both heaps" (#344), holds, twins and the invariant "every runtime
reference that the cycle graph ignores must be an edge from a core
object that marks its target". The identity bugs (#278, #277, #276,
###189, #304, #167/#389, #217) are all *persistent* cross-heap references
going out of sync. A staged mixed heap would make that the state of the
whole runtime for months.

**Why B.** It is how G2 was done successfully: build the new core from
scratch beside, prove it on the gate, measure, then switch.
- *Hill-climbable from day 1.* The conformance harness gains a fourth
  runtime column `rt` with its own ratchet directories
  (`tests/r/passing-rt/`, `tests/corpora/*/passing-rt/`, ratchet rules
  unchanged). Every PR moves a number.
- *No long-lived branch.* `crates/robigo-rt` is new files, so nothing
  conflicts with work on the old runtime. Branch life is zero; every PR
  merges to main.
- *Parallel.* Builtins port module by module (one owner per
  `builtins/*.rs`, docs/workstreams.md protocol), each PR gated by its
  corpus slice in the `rt` column.
- *Both runtimes are measured on the same suites* until the flip, so
  "must not get slower" is checkable per benchmark.

**A'** (a facade over `Rc` with the same lifetimes) would let the
builtins convert on main before the representation changes. It is
viable, but it costs hot-path speed on main while the facade adds an
indirection through `&Value`, and the evaluator core is rewritten anyway.
I would choose it only if B's boot cliff (§11, R1) turns out to be worse
than measured.

##### 9.2 The order of work

| step | where | what | exit |
|---|---|---|---|
| M0 | main | this design, council-reviewed; core.md §12 items 24/25 updated | lead's sign-off |
| M1 | main, `crates/robigo-rt` (not in default build) | the **prototype** (§10): `Cx`, `obj` API, slot envs, closures, promises, contexts, `Flow`, the hybrid segments, a VM subset, enough base for `calls.R` | the §10 exit criteria |
| M2 | main, core | core changes: Rust segment kinds and `collect_explicit`, types 26/27 tracing, weak cache words, symbol segment ids, PROMSXP/ENVSXP unboxed-tag tracing; gctorture covers them | core tests, `release-tests` job |
| M3 | main, rt | evaluator core complete: AST interpreter (bootstrap, `eval`, deopt), VM (port of `compile.rs`'s analyses and op set, new code generation), S3 dispatch, conditions/restarts, `on.exit`, browser hooks. `ROBIGO_BOOT_VERBOSE` under rt lists the missing internals | `robigo-rt --vanilla -e 1` boots base with default packages |
| M4 | main, rt | builtins by module, ported from GNU R's C first with the old Rust port as a reference (§9.4); one PR per module or sub-module, each raising `passing-rt` | `rt` ratchets ⊇ old ratchets for that module's corpus slice |
| M5 | main, capi | capi over rt: `host.rs` callbacks become direct calls, `.Call` passes `Sexp`s, the methods.so hooks (core.md §13) go into rt's dispatch | capi corpus and package suites green in rt |
| M6 | main, rt | Cranelift JIT on the new ABI (§7.4), whole-closure trees | perf.py rt ≤ old on JIT-heavy benchmarks |
| M7 | main | **flip**: rt becomes the default binary. One PR deletes `crates/robigo-runtime`, `crates/robigo-capi/src/bridge.rs` and the twin/hold/proxy core machinery, and moves `passing-rt` markers to `passing` | every marker in every `passing/` also in `passing-rt/`; perf.py: no benchmark slower than the old runtime by >0.5%; G3's gate (design.md §5.6) |
| M8 | main | re-measure item 15 (runtime in libR.so) | ledger entry |

When the bridge dies: in M7, together with the old runtime, never
earlier. The old runtime needs the bridge until it is deleted.

##### 9.3 What pauses on main now, and what continues

- **Pause**: new builtins and Rust ports of stats natives in the old
  runtime (design.md §5.6 already says so); new bridge features (twins,
  holds, deferred ALTREP; only fixes that unblock package ratchets);
  VM/JIT micro-optimizations tied to `Value`/`Env` layout (perf-vm
  "redo, don't port"); the `Value` narrowing and the thread-local move in
  the *old* runtime (step 3); `RString` work (core.md §4.1 already
  decided "no further runtime work").
- **Continue**, because rt reuses it as is: robigo-syntax; robigo-numerics;
  **robigo-text** extraction (formatting, sprintf, regex, encodings as
  pure functions over `&[u8]`/`f64`; architecture.md has it planned and
  it is the best parallel work there is, because both runtimes call it);
  robigo-core (generational prep, item 13 accounting, Arrow ALTREP);
  robigo-capi exports over the core; base natives from GNU R's sources
  (stats, utils, …: they run unchanged on rt); conformance scripts and
  corpora (every new test is a target for rt).
- **Conformance fixes in the old runtime** are allowed only with a
  committed test. That test is then automatically a target for rt, so
  the two runtimes cannot silently diverge.

##### 9.4 Converting 60k lines of builtins and ~4,900 match sites

- **Port GNU R's C, not the old Rust**, wherever the old port's structure
  follows `Value` instead of C. The new API is SEXP-shaped (`car`, `cdr`,
  `view`, `alloc`, `MAYBE_SHARED`), so a C function ports almost line by
  line. That is a porting-contract win (§1 "GNU R's C sources are the
  spec"): the code reads like the C it cites. Most `PROTECT`s simply
  disappear, because allocation keeps `Ref`s alive. A `PROTECT` that
  spans an `eval` becomes a `RootScope`, and the compiler finds every one
  that is missing.
- **A mechanical pre-pass** (a syn-based rewriter in
  `scripts/g3-codemod/`, run per module) for what is pure spelling:
  `Value::Real(v)` → `View::Real(v)` in patterns, `Value::Null` →
  `View::Null`, `v.len()`, `Value::str(..)` → `cx.scalar_str(..)`,
  `RStr` → `Char<'_>`. What it cannot decide (where to root, deferred
  against immediate warnings, in-place decisions) is left as compile
  errors for the porter. This is a guess at the split; measure it on
  the first module in M4.
- **Who**: agents per module (docs/workstreams.md ownership), each with
  the module's corpus filter (`scripts/corpus.py run fastr --filter
  TestBuiltin_<x>` in the `rt` column) as their metric.
- **Boot crutch (optional, measured)**: a reverse bridge for *leaf*
  builtins (pure functions of their arguments: no env arguments, no
  callbacks, no connections) that converts `Sexp` arguments to `Value`,
  calls the old implementation and converts the result back. It would
  let base boot in M3 before every internal is ported. It creates only
  transient values, never persistent cross-heap references, so the
  argument against C does not apply. Use it only if the M3 internal count
  (`ROBIGO_BOOT_VERBOSE`) is large. Each use is listed in one table, and
  the table must be empty before M7.

---

#### 10. The prototype and measurement plan

##### 10.1 Build first (M1, `crates/robigo-rt/proto`)

- `Cx` with an embedded `Core`; the `obj` API (§1) including
  `RootScope`, `Args`, `Out`, `FreshVec`; R-entry segments (§2.2).
- Parser output (robigo-syntax) → LANGSXP cons cells.
- Slot environments (§3.1), with the separate-FRAMESXP variant behind a
  cargo feature for D2.
- Closures, promises (with and without `ARG_FORCED`), argument matching,
  `...`, contexts, `on.exit`, `Flow`, handlers and restarts.
- VM subset: constants, variables (slots), `<-`, `<<-`, `if`, `for` over
  compact ranges, `while`, arithmetic and comparisons on scalars and
  vectors (one kernel each), `[[`/`[[<-` on lists, `$`/`$<-` on lists
  and envs, calls with the call-site cache, `UseMethod`/`NextMethod`.
- Enough base: `list`, `new.env`, `structure`, `cat`, `c`, `length`,
  `stop`/`tryCatch`/`conditionCall` loaded from `rlib/base/R` with the
  internals they need (`.addCondHands`, `.signalCondition`,
  `.dfltStop`, `.addRestart`, `.invokeRestart`), so that `calls.R` runs
  **unmodified**.
- `gc()`, `reg.finalizer`, and weak references through the core.

##### 10.2 Decisions settled by numbers

| id | question | variants | benchmark | metric |
|---|---|---|---|---|
| D1 | rooting | hybrid (§2.2) vs conservative-everything vs precise-everything (handle per read) | fib, calls.R, binarytrees; a finalizer script | instructions per call; finalizer runs at the right `gc()` in 1,000 ASLR runs |
| D2 | frame layout | inline slot block vs env + FRAMESXP vs GNU R pairlist frame | calls.R, fib, `local-fun-calls`-style lookups | instructions, allocations per call |
| D2c | guard memo key | address + generation vs memo cell in the shape | calls.R (guard misses: perf-vm "Next", ~5,000 instructions per iteration) | instructions |
| D3 | VM slots | SoA tags + payload vs 16-byte AoS | fib, `scalarFor`, nbody (scalar-heavy) | instructions, L1 misses (`--cache-sim=yes`) |
| D4 | promise values | unboxed in aux vs boxed scalar | `f(i * 0.5)`, fib | instructions, bytes allocated |
| D4b | caller-forced args | `ARG_FORCED` slots vs forced PROMSXP | fib, calls.R | instructions, allocations |
| D5 | context record | 96-byte `Ctx` in a `Vec` vs GNU R-sized | calls.R, tryCatch row | instructions |
| D6 | link counting | deferred (GNU R's INCLNK) vs eager on register writes | `x[i] <- v` loops (`vectorFor`), `matrixFor`, `l[[i]] <- x` | instructions |
| D7 | isolate access in exports | sole-isolate global vs TLS | `crates/robigo-capi/bench` | per-call instructions |
| D8 | allocation fast path inlined into rt | `#[inline]` across crates vs LTO only | binarytrees | instructions per allocation |

##### 10.3 Metrics and how to take them

- Instructions: `valgrind --tool=callgrind` on the release prototype,
  per call or per iteration computed as N vs 2N iterations (perf-vm's
  method), against main's VM (`ROBIGO_JIT=0`, and with inlining off for
  the general path, where the VM has a switch for it; if not, compare
  `calls.R` rows, which avoid inlining by construction) and against GNU R
  4.5.0 (byte-compiled).
- Allocations: core `GcStats` (count, bytes) per call.
- Peak RSS: `/usr/bin/time -v` on binarytrees (depth 16) and
  `x <- 1:2e8; x[5L]` (G3 gate: constant memory).
- Pauses: core timers around `collect`, reporting max and p99 per run;
  binarytrees and a 2,000-closure-cycle leak script
  (`f <- function(i) { g <- function() i; g }`, design.md §5.2).
- Wall time: hyperfine, 30 runs, on a quiet machine, for `calls.R`.
- Determinism: `weak-keys-collections.R` and the 2026-11-11 stress script
  (`.Call('pointer')` at depths a, b in 0..20), under ASLR, 1,000 runs,
  plus `ROBIGO_GCTORTURE=1`.

##### 10.4 Exit criteria (all, before M3 starts at scale)

- E1: the general call `f(i * 0.5)` with two defaults left unforced costs
  ≤ 0.75x GNU R's instructions and ≤ 0.6x today's 5,597 (perf-vm
  2026-11-10).
- E2: fib with inlining and JIT off ≤ 0.9x GNU R's instructions;
  binarytrees ≤ 0.8x GNU R's instructions with peak RSS ≤ 1.1x GNU R's.
- E3: `calls.R` unmodified, same output, ≤ 0.8x GNU R's wall time
  (today 1.23x: 0.32 s against 0.26 s).
- E4: determinism and gctorture clean, as in §10.3.
- E5: three real builtin modules ported on the API (an arithmetic one,
  a string one such as `paste`/`nchar`, an env one such as
  `assign`/`get`/`exists`) with **no `unsafe`**, each within 1.2x its old
  line count, and reviewed for ergonomics by a council member who did not
  write it.
- E6: the capi corpus cases `eval`, `pairlist`, `unwind` and `extptr` pass
  in the prototype with no bridge (one heap, one identity).

If E1 or E2 fail, the decision table says which variant to try. If the
hybrid's ergonomics fail E5, the fallback is precise rooting with
rusty_v8-style handles (D1's third variant), measured on the same table.

---

#### 11. Risks and open questions, ranked

| # | risk | why it matters | measurement that resolves it |
|---|---|---|---|
| R1 | **Boot cliff**: rt runs no conformance case until base boots, which needs many internals | no signal for weeks; parallel porting stalls | count internals that `ROBIGO_BOOT_VERBOSE` lists under rt at M3 start; if more than ~150, use the leaf reverse bridge (§9.4) and track its table to empty |
| R2 | **Rooting holes**: `Ref`s in Rust heap containers; mistakes in the unsafe evaluator core | use after free, rare and layout-dependent | `tests/no_heap_refs.rs`; a gctorture CI job on the `rt` conformance slice with poisoned free slots; the ledger's `plant` technique (fill dead pointers into frames, assert nothing resurrects) for suspended segments |
| R3 | Refcount and barrier cost on every binding store vs `Rc` clone/drop | loops with object assignment | D6 on `vectorFor`, `listFor`, `matrixFor`, `Storage` |
| R4 | GC behaviour on package suites (heap growth, pauses, collection count) under GNU R's trigger | packages run at 0.7-1.0x GNU R already | the package suite in the `rt` column: RSS, time, `GcStats`; core.md §11 sticky marks if pauses dominate |
| R5 | Derived pointers outside an object (LLVM strength reduction) in the current segment | a vector freed while a kernel reads it | gctorture plus poisoning on the vector-kernel corpus slices; `objdump` audit of the hottest kernels for negative offsets beyond the header |
| R6 | Deferred warnings differ from GNU R for a builtin misclassified as pure | message order, `warn = 2` side effects | the source scanner's `pure` list; corpus diffs on `TestBuiltin_*warning*`; a test per side-effecting builtin with `withCallingHandlers(warning = function(w) stop())` |
| R7 | Finalizer timing of allocation-triggered collections | package tests that count finalizer runs without `gc()` | grep the package suites for `reg.finalizer` tests without an explicit `gc()`; if any depend on timing, make `R_gc_running` collections use `collect_explicit` for their final pass |
| R8 | Divergence: fixes landing in the old runtime during M3-M7 | rt misses behaviour that only old code has | the "fix needs a test" rule; at M7 the marker superset check is mechanical |
| R9 | Aliasing UB between `&[T]` and C writes that do not go through `&mut Cx` (a C thread, signal handlers) | undefined behaviour Miri cannot see | an audit: no package C code runs except under `&mut Cx` (finalizers, ALTREP methods, connections, all through the trampolines of core.md §5); C threads must not touch R objects (GNU R's rule too) |
| R10 | JIT rewrite lags and the flip waits | perf regressions at M7 | M6 gate per benchmark; the VM alone must already meet design.md's speed table |
| R11 | Symbol segment as one virtual range (`id` arithmetic) | address-space reservation on wasm32 | wasm32 build in G5; fallback: the id in the symbol's `extra`+`refcnt` words (symbols are immortal, their refcnt is unused) |
| R12 | Lazy `promargs` and `ARG_FORCED` materialization miss a reader | `match.call`/`substitute`/`sys.function` differences | `tests/r/basics/caller-forced-promises.R` (exists) in the rt column; `ROBIGO_MATERIALIZE_TORTURE=1` makes every call materialize eagerly, and the corpus must match in both modes |

Open questions for the council:

1. Is the deferred-warning rule (§4.5) acceptable under "messages are
   verbatim" and "port behaviour"? I believe it is unobservable for pure
   builtins and save `warning_now` for the rest. A stricter council
   could make every warning `&mut` at the cost of rooting in each
   warning path (cold paths, so the cost is code, not time).
2. Inline slot frames (D2) make ENVSXP variable-size. Is any C API
   (or `Defn.h` macro) assuming `sizeof` an environment? I found none
   (core.md §2: "no package imports FRAME or HASHTAB"); confirm against
   `tests/capi/symbols.tsv`.
3. `ROBIGO_SOLE` (§8) is process-global mutable state, which core.md §10
   forbids in libR. I argue that a read-mostly pointer to the single
   isolate is a cache of the TLS value, not state. The lead decides.


### Muse

### G3 design: robigo's runtime rebuilt on robigo-core

Muse Wonder Full, design council. Base `4cff0ceb`. Read-only review of
`AGENTS.md`, `docs/architecture.md`, `docs/design.md` §3/§5, `docs/core.md`,
`docs/ledger/{perf-vm,core,memory}.md`, and the code named in the brief.
Citations are `path:line` in this tree unless marked GNU R. Guesses are
marked *(guess)*.

#### 0. Verdict in one page

The coordinator's direction is right — one heap, one identity, bridge
deleted, prototype first — but three parts of the plan and two settled
core decisions are wrong for G3 and will cost more if discovered late:

1. **Do not conservatively scan Rust frames.** `docs/architecture.md:117-123`
   and `docs/core.md` §3 assume the bake-off's whole-stack scan carries
   over ("Rust code holds `Sexp`s as plain `Copy` pointers"). It does not.
   It is already the documented cause of nondeterministic finalizers
   (`docs/ledger/core.md:2260`, `docs/core.md:269-286`) and of the
   2026-11-11 stale-word saga (`docs/ledger/core.md:7-158`), and it is
   unsound in the other direction under LLVM: a live `Sexp` held only in
   a register, or only as a derived `base+offset` value with the base
   dead, is invisible to a word scan. G3 rooting is **precise for Rust,
   conservative only inside C segments** — the exact split the bridge
   was forced into, generalised (§2 below).
2. **Do not "narrow `Value`'s interface on main" as the main prep.** A
   `view()` over today's `Rc` enum (`crates/robigo-runtime/src/value.rs:885`)
   cannot express the borrowed-read rules of §1, so most of that churn is
   thrown away at the swap. The layout-independent prep that *does*
   survive is the isolate record (§8), keeping `Sym` a `u32` (§5), and a
   mechanical census of the ~6,800 `Value::` occurrences (my count:
   `rg "Value::" crates/robigo-runtime/src -o | wc -l` = 6821; the brief's
   ~4,900 presumably counts distinct match/constructor sites) so the
   codemod in §9 is sized before anyone starts.
3. **No long-lived integration branch.** Drift killed pqR and Rho
   (`docs/design.md:39-42`); this repo lands many runtime PRs a week. The
   swap branch must live for days, which is only possible if the
   conversion is mostly mechanical (§9): keep the *name* `Value` as a
   `Copy` wrapper over `Sexp` with the same constructors, so the diff at
   most sites is "add the isolate argument", not a rewrite.
4. **Core change A: environments stop being 48-byte nodes.** Council
   item 20 fixed only `ENCLOS` (+24) for C (`docs/core.md:112,128-139`;
   `crates/robigo-core/src/lib.rs:174-180,777-848`). Use that freedom now:
   an 80-byte ENVSXP body with inline version counters and binding masks
   (§3). Retrofitting versions after builtins depend on env layout is
   the costly path.
5. **Core change B: a construction (builder) API.** Every heap store
   from outside core pays `Sexp::store` — barrier test, `tracks_refs`
   type load, two refcount updates (`crates/robigo-core/src/lib.rs:605-703`).
   Filling a fresh million-element `VECSXP` through `set_elt` in a loop
   would be a self-inflicted regression against today's `Vec` push.
   Fresh, unpublished objects need an unchecked fill path (§1.4).

Everything else in the coordinator's plan I endorse, in particular **no
mixed Rc/GC heap as a steady state** (§9.3) and **measure the prototype
before the rebuild** (§10) — with sharper exit criteria than "vs today's
VM on fib": the prototype must also prove deterministic finalization
under `ROBIGO_GCTORTURE=1` and the general-call path of
`benchmarks/suite/calls.R`, because that is where robigo loses today
(1.1–1.8×, `docs/ledger/perf-vm.md:40-51`) and where slot frames,
caller-forced promises and rooting discipline all collide.

New/changed files this design names:

| File | Change |
|---|---|
| `crates/robigo-runtime/src/obj.rs` | new: `Value`, `View`, typed vector views, builders — the council item 24 spec as code |
| `crates/robigo-runtime/src/isolate.rs` | new: `Isolate` (alias `Cx`), absorbs `Interp` + capi `State` |
| `crates/robigo-runtime/src/roots.rs` | new: handle scopes, `Rooted`, root-stack registration |
| `crates/robigo-runtime/src/env.rs` | rewritten on ENVSXP (§3); `env/table.rs`, `env/trace.rs`, `autogc.rs`, `reachability.rs` deleted |
| `crates/robigo-runtime/src/value.rs`, `altrep.rs` | deleted (contents move to `obj.rs` / core ALTREP) |
| `crates/robigo-core/src/{lib,gc}.rs` | `alloc_env` (80 B), builder stores, env trace, offsets exported for JIT |
| `crates/robigo-capi/src/bridge.rs` (5,971 lines) | deleted; `host.rs` callback table deleted with it |
| `experiments/g3proto/` | new throwaway prototype workspace (§10), like `experiments/memory` |

#### 1. The Rust-side object API (council item 24)

##### 1.1 `Value` becomes a `Copy` wrapper over `Sexp` — no lifetime brand

```rust
// crates/robigo-runtime/src/obj.rs
###[repr(transparent)]
###[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub Sexp);            // 8 bytes; the pointer C sees

pub type Env = Value;                  // with typ()==ENVSXP checked at construction sites
pub struct CharSxp(pub Sexp);          // CHARSXP; NA is cx.na_string(), not None
```

I reject lifetime-branded handles (`Value<'gc>`) as the primary type.
Branding is sound only if no handle ever escapes its brand, but R values
are stored *in the heap itself*, handed to C, kept in VM registers
across calls, and cached in call sites (`crates/robigo-runtime/src/compile.rs:636-692`).
A brand would have to be `'static` at every interesting boundary,
infect ~115k lines of signatures, and still say nothing about C writes
or ALTREP materialization. Branding belongs on **views** (§1.3), which
are short-lived by construction.

Keeping the name `Value` and its constructor surface is deliberate:
today's `Value::real(x)`, `Value::int_fresh(x)`, `v.ptr_eq(w)`,
`v.same_object(w)` etc. become `Value::real(cx, x)` (one 48-byte
allocation, `docs/core.md:162-163`), address equality, and so on. The
enum variants at `value.rs:885-907` disappear; matching goes through
`view()` (§1.2). `Builtin` values are real BUILTINSXP/SPECIALSXP nodes
whose car word is the builtin's index (GNU R's offset encoding,
`docs/core.md:337-339` on TRACKREFS), made once at startup — this also
gives C a true `.Primitive` object for free, where today the bridge
proxies them (`docs/core.md:607`).

`Sym` stays a `u32` (§5), so `Value::Sym(Sym)` sites become
`cx.symbol(sym)` — one load from the isolate's symbol vector.

##### 1.2 Typed views for matching

```rust
pub enum View<'a> {
    Null,
    Sym(Sym),
    Lgl(RVec<'a, i32>), Int(RVec<'a, i32>), Real(RVec<'a, f64>),
    Cplx(RVec<'a, Rcomplex>), Raw(RVec<'a, u8>),
    Str(RVec<'a, CharSxp>), List(RVec<'a, Value>), Expr(RVec<'a, Value>),
    Char(CharSxp),                       // a bare CHARSXP (C can hand one over)
    Pairlist(Value), Lang(Value),       // cons spines: walk with car/cdr/tag
    Closure(Value), Builtin(&'static Builtin), Env(Value),
    Promise(Value), Dots(Value),        // DOTSXP is a pairlist of promises (§4)
    ExtPtr(Value), WeakRef(Value), Obj(Value), Bytecode(Value),
    Altrep(Value),                      // any vector type with the ALTREP flag
}
impl Value { pub fn view<'a>(self, cx: &'a Cx) -> View<'a>; }
```

`view()` reads the type byte (+0) and flags byte (+1)
(`crates/robigo-core/src/lib.rs:253-300`) — two loads, no allocation, no
ALTREP method call. An ALTREP vector reports as `Altrep` plus its
vector type, *never* silently as a slice: code that wants elements
calls `cx.elt_real(x, i)` / `cx.real_region(x, ..)` (which dispatch
like GNU R's `REAL_ELT`/`Get_region`), and only `cx.dataptr_ro(x)` /
`cx.dataptr_rw(x)` may materialize, mirroring capi's existing split
(`crates/robigo-capi/src/altrep.rs:347-360`). This fixes by construction
the known gap at `docs/core.md:803-804` ("forcing reads every element
where GNU R's `length()` calls only `Length`"): `length` on an ALTREP
view is a `Length` method call, not a slice length.

For the common builtin shape, typed extractors replace match arms:

```rust
let xs: RVec<f64>   = cx.reals_ro(arg)?;        // coerce-free, type error verbatim
let xs: RVecMut<f64>= cx.reals_rw_unique(arg)?; // §1.4: duplicates if shared
```

##### 1.3 Borrowed reads: the safepoint rule, enforced by `&mut Cx`

`RVec<'a, T>` derefs to `&'a [T]`; `Chars<'a>` to `&'a [u8]`. Three
things can invalidate such a slice, and the design treats them
differently — conflating them is how the current `*_through_alias`
hazards (`docs/architecture.md:158-159`) arose:

| Threat | Real? | Rule |
|---|---|---|
| GC moves/frees the object | No move (non-moving heap, `docs/core.md:249-250`); free only if unrooted | The viewed `Value` must be rooted/reachable for `'a` (§2); the address itself is stable |
| Allocation triggering GC | Only a problem via freeing (above) | Covered by rooting; but allocation is still a *safepoint* because of the next two |
| ALTREP materialization, `SETLENGTH`, C writes through `REAL(x)`, `duplicate` on modify | Yes: data pointer/length/contents can change under a live slice; a `&[f64]` also asserts no concurrent mutation to LLVM | **No safepoint may run while a view is live** |

The enforcement mechanism is the borrow checker, not a comment:
**every operation that can allocate, evaluate, call C, call an ALTREP
method, warn (warn=2 unwinds), or duplicate takes `&mut Cx`, and every
view borrows `&Cx`.** A live `&[f64]` therefore statically blocks all
safepoints in the same function. This is the single most important
signature decision in G3, and it is why `Interp`'s current omnipresent
`&mut self` (`crates/robigo-runtime/src/builtins/mod.rs:63`) must be
split: pure accessors take `&Cx`, effectful operations take `&mut Cx`.
Code that genuinely needs a safepoint mid-kernel (coercion warnings,
`Get_region` chunking) drops the view, runs the safepoint, and
re-acquires with `cx.revalidate(view)` which re-reads type, flags,
length and data pointer — the pattern capi already uses informally
around bridge write-backs, now typed.

Two corollaries. (a) Interior `RefCell`/`Cell` state in `Cx` must be
limited to caches whose mutation cannot allocate or run R code;
anything else behind a `RefCell` reachable from `&Cx` re-opens the
hole. (b) `unsafe` is confined to `obj.rs` plus core/capi: constructing
a slice from `std_data()`+`std_len()`
(`crates/robigo-core/src/lib.rs:461-508`) is `obj.rs`'s job, exactly as
`sys.rs` confines POSIX today (`docs/architecture.md:137-138`).
`robigo-runtime` keeps `#![deny(unsafe_code)]` with `obj.rs` named.

Debug enforcement: under `debug_assertions`, `Cx` carries an epoch
bumped at every safepoint and views carry the epoch they were taken
at; `Deref` asserts equality. This catches safepoints smuggled through
`&Cx` interior mutability in tests. Cost in release: zero. *(The
debug-epoch mechanism is my proposal; unmeasured.)*

##### 1.4 Mutation, copy-on-modify, and builders

Copy-on-modify is GNU R's REFCNT protocol, already implemented in core
(`docs/core.md:345-352`; `crates/robigo-core/src/lib.rs:746-774`):

- `MAYBE_SHARED(x)` = `refcnt > 1`. Unlike today's `Rc` strong count,
  locals, VM registers and the protect/root stacks do **not** count —
  only heap-to-heap stores and bindings do (via `Sexp::store`). This is
  *less* conservative than `Rc::make_mut` in one direction (a value in
  two registers is still unique — good for `x[i] <- v` in loops) and
  *more* in another: the sweeper never decrements a dead parent's
  contribution (`docs/core.md:350-352`), so counts only ever overstate.
  Builtins must therefore not treat `refcnt == 1` as ownership proof
  for anything except the duplicate decision; in particular the
  `Rc::get_mut`-style "move out of the binding" trick
  (`docs/ledger/perf-vm.md:2080-2081`) is replaced by: take the `Value`
  (a `Copy` load), `cx.ensure_unique(x, call)` duplicates iff
  `MAYBE_SHARED || ALTREP || EXTDATA-with-other-owners`, then mutate.
- Replacement builtins keep today's by-value discipline
  (`AGENTS.md:68-72`): `args` is a rooted `Vec<Arg>`; the target is
  *taken* with `mem::replace(&mut arg.value, cx.nil())` so the VM's
  in-place window (`Interp::replacing`, `interp.rs:251-252`) still
  works. Validation-before-mutation stays mandatory: with refcounts, a
  half-mutated then errored object is observable through its other
  references exactly as in GNU R — but tracemem-visible copies
  (#140) must match GNU R's `duplicate` sites, so `ensure_unique`
  takes the call and reports through the same hook as capi's
  `watch_write` does for twins today.
- **Builders (core change B).** `Core::alloc_vector` already prefills
  STRSXP/VECSXP with nil/blank (`docs/core.md:356-358`), so a fresh
  vector is GC-safe at every instant. Add to core, for objects whose
  address has not been published (not stored into any traced field,
  not returned to C):

```rust
// robigo-core: unchecked fill of a fresh object; refcounts fixed in bulk
pub unsafe fn fill_elt_new(x: Sexp, i: usize, v: Sexp);   // incref only, no barrier/old-value decref beyond the prefill
pub unsafe fn data_mut_new<T>(x: Sexp) -> *mut T;          // numeric payload of a fresh owned vector
```

  `obj.rs` wraps these as `cx.build_vec(n, |b| ...)` /
  `b.set(i, v)` / `b.data::<f64>()` where the builder holds `&mut Cx`,
  so no safepoint can publish the object mid-fill. This is the
  equivalent of GNU R's `allocVector` + raw `REAL()` fill that every
  C builtin port assumes. The 142 `Rc::make_mut` sites (127 by my
  count) map to `ensure_unique` (shared case) or builder fill (fresh
  case); the codemod (§9) must distinguish them — that distinction is
  the hand work.

##### 1.5 What builtins look like after

```rust
pub type BuiltinFn = fn(&mut Cx, call: Value, args: Vec<Arg>, env: Value) -> RResult<Value>;
// Arg { tag: Option<Sym>, value: Value } — unchanged shape, Sexp inside
```

Same `prim/special/internal` registration (`builtins/mod.rs:164-176`),
same `match_args`/`check_arity` helpers re-typed. A builtin that only
reads can downgrade `&mut Cx` to views immediately; the signature
stays `&mut` because almost every builtin allocates its result.

#### 2. Rooting Rust frames: precise handles, conservative C only

**Decision: hybrid with precise primary.** Rust frames root precisely
through handle scopes on an isolate root stack; the conservative scan
survives *only* for C segments, using the segment machinery that
already exists (`Core::close_scan_segment`,
`crates/robigo-core/src/gc.rs:1114-1136`; scan at `gc.rs:1460-1477`),
including the x86-64 naked stub and 1 KiB zeroing
(`docs/core.md:277-286`). Whole-stack conservative scanning of Rust
frames is rejected. How the named systems actually do it:

| System | Mechanism | Lesson for G3 |
|---|---|---|
| GNU R (`memory.c:PROTECT`) | precise explicit stack; C locals unprotected at their own risk | our C side already *exceeds* this via the segment scan; keep for C |
| V8 (`HandleScope`, `Local<T>` = pointer to a handle slot) | precise; handles are slots in scoped blocks; no raw pointers across allocations | the model to copy for Rust: scoped slots, cheap push |
| JavaScriptCore | conservative stack scan — but JSC controls its compiler, tags every `JSValue`, and spills registers at GC points by construction | we control none of those: rustc/LLVM keep values in registers or derived forms with no stack map |
| Julia (`jl_gc_frame` / `JL_GC_PUSH`) | precise explicit frame chains linked through a task-local, generated code maintains them | closest to our VM/JIT register-file problem (§7) |
| CPython | refcounts (the bake-off's loser, `docs/ledger/memory.md:85-91`) | — |

Three repo-specific nails in the conservative-Rust coffin, all already
paid for once: deterministic finalizers break (R6 under rlang,
`docs/core.md:275-277`); stale words resurrect dead objects across
profile/compiler changes (`docs/ledger/core.md:7-158`, incl. the
`Value` padding-byte vector at :62-71 — a 16-byte `Value` copy left
pointer-shaped padding that a scan read for a whole `.Call`); and the
collect-on-every-allocation torture mode (`docs/core.md:299-306`)
cannot distinguish "rooted" from "accidentally retained", so
soundness bugs would pass CI.

##### 2.1 The root stack and handle scopes

```rust
// crates/robigo-runtime/src/roots.rs — inside Isolate
roots: Vec<Sexp>,                  // the Rust root stack (NOT R_PPStack)
pub struct Scope { top: u32 }      // cx.scope(|cx, s| ...); Drop truncates — but see unwind rule
pub struct Rooted(u32);            // index into roots; cx.get(r) -> Value
impl Cx { pub fn root(&mut self, s: &mut Scope, x: Value) -> Rooted; }
```

- Separate from `R_PPStack`: C code may `UNPROTECT(n)` across a
  callback into Rust, and GNU R's contract lets it; it must never pop
  Rust roots. Core gains the isolate's `roots[..]` as one more precise
  root range beside PPStack/precious (`docs/core.md:252-261`), fed
  through the existing `Core::root_fns` (`gc.rs:170,535-536`) or a
  dedicated `set_rust_roots(*const Sexp, *const usize)` slot — the
  latter avoids a callback per collection and is my preference.
- `Drop`-based scope exit is **not** sound across `longjmp`: a C jump
  that lands in a trampoline skips Rust destructors. So scopes are
  restored the way `errors::Ctx` restores `R_PPStackTop` today
  (`docs/core.md:482-485`): every context record (§4) saves `roots.len()`
  and `in_context` truncates on unwind. `Scope`'s `Drop` is then only
  the normal-path fast exit; correctness never depends on it. The
  existing `tests/no_inner_jumps.rs` discipline (`docs/core.md:470-474`)
  extends to the runtime crate unchanged.
- The discipline, stated once: a bare `Value` in a Rust local is valid
  **until the next safepoint** (any `&mut Cx` operation that can
  allocate/evaluate/call out). To carry a value across a safepoint,
  root it, store it in a traced heap field, or keep it in a registered
  container (§2.2). This is V8's rule verbatim; `gctorture` + slot
  poisoning makes violations fail immediately instead of rarely.
- Cost control: rooting is a `Vec` push (amortised, no allocation in
  steady state; the vector is pre-grown per isolate). Hot code roots
  in bulk: argument lists are built directly in a rooted `Vec<Arg>`
  (§4), VM registers are a registered container (§2.2), and the
  interpreter opens one scope per `eval` of a call node, not per
  subexpression — subresults that need to survive the next argument's
  evaluation are pushed then. *(Guess: ≤2% on `calls.R`; the prototype
  measures this first, §10.)*

##### 2.2 Registered containers (no per-element rooting)

Per the existing rooting rule (`docs/core.md:288-299`), every
off-stack `Sexp` container registers or is a heap object. G3's list,
each marked by one isolate `root_fns`-style walk or a core root range:

- VM register files + variable slots of every live VM activation (§7);
- the context stack (§4): call, function, cloenv, sysparent, promargs,
  on.exit expressions, handler/restart condition objects;
- `Code::consts` and `CallSite` memos (`compile.rs:782-798,636-660`) —
  or better, constants move *into* the closure's code cell as a
  traced VECSXP (§4), deleting this root class;
- JIT Dyn frame slots (§7);
- isolate globals (global/base/empty/registry envs, options values,
  the symbol vector itself though symbols are immortal);
- the bridge's tables — deleted with the bridge (§9.4); their rooting
  complexity (`PROXIED`, `PINNED`, twins-as-holds,
  `crates/robigo-capi/src/bridge.rs:90-180`) is the strongest single
  argument that the mixed heap cannot be the steady state.

#### 3. Environments

##### 3.1 Layout: ENVSXP grows to 80 bytes

Core change A. `Core::alloc_env` replaces `alloc_node(ENVSXP, ..)`
(`gc.rs:621`); the tracer's node arm (`gc.rs:1349`) splits ENVSXP out.
C compatibility is preserved because only `ENCLOS` at +24 is a fixed
offset (`crates/robigo-core/src/lib.rs:177`) and FRAME/HASHTAB are
shim functions (`lib.rs:812-847`), never field reads by packages.

```text
+0   header (16 B, as all objects; gp carries ACTIVE_BINDING-free bits, extra: RDEBUG/RTRACE)
+16  Sexp  frame_cache   materialised FRAME pairlist, or R_NilValue = dirty/not built
+24  Sexp  enclos        ENCLOS (fixed; OFF_ENCLOS)
+32  Sexp  frame_data    slot mode: VECSXP of slot values · general mode: HASHTAB VECSXP or nil
+40  Sexp  aux           slot mode: descriptor cell (EXTPTRSXP to FrameDesc) · general: nil
+48  u32   fn_version · u32 bind_version      (VM guard keys; today's Cell<u32> at env.rs:423)
+56  u64   missing_default_mask               (slot mode, ≤64 slots; §4)
+64  u32   cache_stamp · u16 nslots · u8 mode (0=pairlist,1=hashed,2=slots) · u8 flags (locked, search, hashed-ever)
+72  u64   id            stable identity for VM memos (today EnvData.id, env.rs:420) — address would do; id survives in debug prints
```

Sizes reported to R are unchanged (GNU R accounting,
`docs/core.md:140-143`). Heap cost: 80 B vs 48 B per call frame — at
fib's measured ~1 env/call this is the one place G3 starts behind;
§10's prototype must show the slot fast path repays it.

##### 3.2 Two frame modes, one-way promotion

**Slot mode** (closure activation frames only): `frame_data` is a
`VECSXP` of values in descriptor order; the descriptor
(`FrameDesc { syms: Rc<[Sym]>, index: HashMap<Sym,u16>, formals_len }`,
built once per closure by the compiler from formals + statically
known locals, owned by the closure's code cell, §4) maps symbols to
slots. Compiled code addresses slots by *index* (`compile.rs` `Slot`
already exists for variables) — variable read in a slot frame is
`VECTOR_ELT(frame_data, i)` behind one type+mode test, no hash, no
`RefCell` borrow (today: `Frame::get` → mask test → linear/indexed
scan under a borrow, `env.rs:63-110,218-240`). Uncompiled lookup uses
the descriptor index (one hash of a `u32`).

**General mode** is GNU R's representation, kept bit-for-bit in
behaviour: pairlist frame (bindings prepended by `defineVar`, initial
frame in order — the `nbase` ordering rules at `env.rs:85-101` carry
over as pairlist construction rules) and, when hashed
(`R_NewHashedEnv`, namespaces, global after `env::table` thresholds),
a HASHTAB `VECSXP` of pairlist chains so `HashTableNames` order
(`env.rs:284-291`) and `R_lsInternal3` match GNU R. All
search-path, namespace, package and C-created environments are born
general; capi's `Rf_NewEnvironment` (`crates/robigo-capi/src/envir.rs:99-102`)
already builds pairlists and keeps doing so, now into a real ENVSXP
instead of a host proxy.

**Promotion** slot→general happens once, irreversibly, on: binding a
symbol not in the descriptor (`assign()`, `eval` creating locals),
`makeActiveBinding`/`lockBinding` on any slot, >64 slots needed,
`rm()` of a slot (simpler than tombstones + mask surgery; `rm` in a
hot frame is rare — *(guess, to be checked against the package
corpus)*), and any C access through `FRAME()`/`SETCAR`-on-env, since
C may then mutate cells the slot vector would not see (the twins
lesson, `docs/core.md:660-684`). Pure R reflection (`ls`, `as.list`,
`get`, `parent.frame`) reads slots *without* promoting: listing walks
descriptor order but must reproduce GNU R's order — descriptor order
is formals-then-locals in binding order, and GNU R lists a closure
frame's later bindings in reverse `defineVar` order; the listing code
reverses the post-formals segment, exactly the transform
`frame_names` does today (`env.rs:284-291`). `frame_cache` (+16) is
built lazily for C and invalidated by bumping `bind_version` on every
define/remove in either mode.

Binding flags: in general mode they live in the binding cell's `gp`
(`ACTIVE_BINDING 1<<15`, `BINDING_LOCK 1<<14`, `docs/core.md:93-96`) —
GNU R's place, so capi needs no translation. In slot mode the only
per-slot flag that can exist is missing-default (mask at +56);
active/locked force promotion, by design. Environment locking
(`lockEnvironment`) is the `flags` byte at +64. The base/base-namespace
shared-frame hack (`FrameCell::Shared`, `env.rs:406-411`) becomes what
GNU R does: two ENVSXPs whose general-mode `frame_cache`/HASHTAB
point at the *same* pairlist/hashtab objects — heap sharing instead
of `Rc<RefCell>` sharing, visible to C identically.

##### 3.3 Versions, global cache, `<<-`

- `fn_version` (+48) bumps exactly where `Env::bump_fn_version` does
  today (`env.rs:905-906` call sites); `bind_version` bumps on every
  define/remove. VM guard keys keep their shape
  (`(search_epoch, env, fn_version)`, `docs/ledger/perf-vm.md:2070-2074`)
  with `env` now the Sexp address — stable for the object's life
  (non-moving heap), unlike `EnvData.id` bookkeeping.
- `search_epoch` / `r_epoch` move from `thread_local!` Cells
  (`env.rs:124-152`) into the isolate (§8).
- **Global cache:** port GNU R's `R_GlobalCache` (GNU R
  `envir.c:616-772`): an isolate hash map `Sym -> (env, binding cell
  or slot, bind_version)` consulted by `findVar`/`findFun` when the
  walk reaches the search path, invalidated wholesale by
  `search_epoch` (simpler than GNU R's per-entry flush, and our epoch
  already exists for `Interp::fun_cache`, `interp.rs:222-230,876-879`,
  which this replaces — one cache, both functions and variables,
  keyed also by starting-env `cache_stamp` for the local-frame prefix
  where `call_site_fun_fast` (`vm.rs:5672`) needs it).
- `<<-` (`do_assignSuper`): walk `enclos` chain; per env, slot mode →
  descriptor probe + `store` into the slot vector + version bumps;
  general → `findVarInFrame` + `SETCAR` through core's `store`. The
  VM's `SuperSet` fast path and its loop-JIT treatment
  (`docs/ledger/perf-vm.md:171-185`) carry over with the slot index
  replacing the binding pointer.

#### 4. Closures, promises, argument matching, contexts

##### 4.1 Closures and the code cell

CLOSXP is a 48-byte node in GNU R's field assignment: `car` +16
FORMALS (pairlist), `cdr` +24 BODY, `tag` +32 CLOENV, `aux` +40 the
code cell (traced, no C accessor, `docs/core.md:117-127`). The code
cell is an EXTPTRSXP whose untraced address word
(`crates/robigo-core/src/lib.rs:710-713`) owns a
`Box<CodeCell>`, released by the sweeper's release mechanism at a safe
point (the HOST/EXTDATA release path, retired for this use):

```rust
struct CodeCell {
    code: OnceCell<Option<Rc<Code>>>,        // today's Closure::code (value.rs:625-631)
    inline: OnceCell<Option<Rc<Inline>>>,
    defaults: OnceCell<Rc<[Option<Rc<Code>>]>>,
    desc: OnceCell<Rc<FrameDesc>>,           // §3.2 slot descriptor, from formals+locals
    consts: Sexp,                            // VECSXP of Code::consts — traced, not a Rust-root class
    jit: JitState, native: NativeSlot,       // as jit/closure.rs today
}
```

Allocated lazily on first call/compile, so an uncompiled closure is
one 48-byte node (today: an `Rc<Closure>` plus its `Rc<[Arg]>` formals
plus the body value — G3 is strictly smaller). Retiring native code
in a dead cell must wait until no JIT frame can be inside it: release
queues into the isolate's retire list drained at top level, the same
rule the bridge uses for C-replaced bodies (`docs/core.md:520-523`).
The same EXTPTR-cell trick on LANGSXP `aux` holds the interpreter's
call-site cache (resolved function + `(search_epoch, fn_version)`
stamp), retiring `si_expr_memo`'s weak-key tables (`interp.rs:212-222`)
for compiled-out call sites; VM `CallSite`s stay in `Code` (§7).

BCODESXP (GNU R bytecode from serialization) keeps today's
`Bytecode` payload (`value.rs:948-975`) inside a code cell on the
BCODESXP node; execution still decompiles to the expression, as now.

##### 4.2 Promises: heap promises are boxed; unboxed scalars never reach the heap

PROMSXP node, GNU R fields: `car` PRVALUE (`R_UnboundValue` until
forced), `cdr` PRCODE (expression), `tag` PRENV (nil after forcing),
`gp` low bits = PRSEEN (0/1 forcing, 2 interrupted — porting
`Promise::forcing/interrupted`, `value.rs:644-666`,
`docs/ledger/perf-vm.md:62-66`), `aux` = bytecode for PRCODE when the
creating code was compiled (a BCODESXP/code cell reference; nil for
defaults, `delayedAssign`, lazy-load — the `bc`/`default` distinction
at `value.rs:660-666` becomes `aux != nil` plus one `extra` bit).

`PromiseVal::Scalar*` (`value.rs:636-642`) does **not** survive as a
heap representation: there is no untraced word in a 48-byte node to
hold raw `f64` bits (§1.1 of core layout), and encoding bits as a
fake `Sexp` in a traced field corrupts the collector. Instead,
unboxed promise values live where unboxed values already live — in
VM registers and in the *promargs array* (§4.3):

- Caller-forced promises (`vm::caller_promargs`,
  `docs/ledger/perf-vm.md:2062-2067,2091-2142`) are entries in the
  rooted promargs array carrying an `RVal`; they are materialised
  into a PROMSXP (PRVALUE boxed, PRENV nil, as GNU R's forced
  promise) only when something needs the node: binding into a
  general-mode frame, escaping into a closure/`...`/list, C reading
  the binding (`findVar` — generalising today's
  `interp::materialize_default`, `docs/core.md:831-837`), or
  `substitute`/`missing` on a frame that has been materialised.
  In a slot frame, a caller-forced scalar is written to the slot as
  its box *once* if the slot is ever read as an object
  (`Env::binding_value`'s contract, `docs/core.md:628-633`, becomes
  `cx.binding_object(env, slot)`); the VM's slot cache (`Vars`)
  keeps the `RVal` and flushes as today.
- Constant defaults: bind the constant + set the
  `missing_default_mask` bit (§3.1), exactly today's
  `BIND_MISSING_DEFAULT` semantics (`env.rs:16-17`,
  `docs/core.md:825-837`), materialised to a real unforced PROMSXP on
  first C/reflection look. The recorded capi difference (forced-ness
  visible after an R read) is fixed cheaply: the VM slot flush marks
  the mask bit's neighbour state by *materialising then forcing* if
  the slot was read as an object before C looks — one bit in `Vars`
  per formal. *(Detail to verify against the `default_args` capi
  case; if it costs on the hot path, keep the documented gap.)*
- Forcing a heap promise from the VM returns an `RVal` and boxes into
  PRVALUE only on flush/escape — `force_promise_unboxed`
  (`interp.rs:692`, `docs/ledger/perf-vm.md:52-61`) with its write-back
  deferred to the same points that flush `Vars`. `PRSEEN=1` during
  forcing gives GNU R's "promise already under evaluation" and the
  interrupted-restart warning for free, in the node C also sees —
  #277 (C-built promise not forced) is impossible when there is one
  node.

##### 4.3 Argument matching and `...`

Promargs during a call are a **rooted Rust array**, not a pairlist:

```rust
struct PromArg { tag: Option<Sym>, prom: Prom }   // in isolate's recycled Vec (today's arg_pool, interp.rs:226)
enum Prom { Node(Value), Forced(RVal, /*expr*/ Value), Missing }
```

The pairlist GNU R builds (`promiseArgs`) is materialised only when
observable: `match.call`, `sys.call` reconstruction, `...` binding,
C's `R_execMethod`/`promargs` access, or a general-mode frame.
Matching (`interp.rs:1237` `match_args`, GNU R match.c three passes)
runs on the array with three fast paths in front, in order:
(1) **positional-direct** — all args positional, count ≤ formals, no
`...` capture needed: bind slots directly (today's `bind_positional`,
`interp.rs:2263`); (2) **exact-tag** — every tag matches a formal
exactly via the descriptor index (today's `bind_exact`,
`interp.rs:2288`); (3) general three-pass over the array, then bind.
Defaults bind per §4.2. `...` binds as a DOTSXP: a node whose `car`
is the materialised pairlist of PROMSXPs (GNU R's shape), built at
bind time only if the callee frame is general-mode or `...` is read;
in slot mode `...` occupies a slot holding that DOTSXP (built lazily
on first `..n`/`...` read, cached in the slot — `list(...)` then
forces in order through the nodes). This keeps `lapply`-style dots
traffic allocation-light while making `..1` after C has poked the
dots pairlist (`node_poke_car` etc., `docs/core.md:663`) trivially
correct — there is only one spine.

##### 4.4 Contexts, `Flow`, and C ↔ Rust ↔ C

The Rust context stack keeps today's shape (`interp.rs:51-77`) with
`Value`/`Env` fields re-typed to `Sexp`, living in the isolate and
registered as roots (§2.2), plus per-context saved `roots.len()`,
`R_PPStackTop`, handler/restart depths — capi's `errors::Ctx``
(`docs/core.md:482-485`, council item 3) merges into it, deleting the
host `ctx_save/restore/drop` callbacks (`crates/robigo-capi/src/host.rs:41-47`).
`sys.function/sys.call/sys.parent/on.exit` read this stack as today.

For C, `RCNTXT` (GNU R `Defn.h:1319`) is **published lazily**: capi
maintains a mirror chain in the isolate, (re)built from the Rust
stack at each entry into package code and invalidated on context
push/pop (a generation counter). Code that only reads
`R_GlobalContext->call` pays one chain walk per native call, not a
dual-stack tax per closure call. *(If a package is found to cache an
`RCNTXT*` across calls — none known; check methods.so's imports,
`docs/core.md:1079-1093` — promote that mirror to eagerly maintained
for the affected context kinds.)*

`Flow` (`crates/robigo-runtime/src/error.rs:20-26`) stays the Rust
unwind currency; core's `Unwind` token becomes an index into a single
isolate slot `pending: Option<Box<Flow>>` plus the continuation node
owning it (today: bridge `FLOWS` hashmap + HOST proxy,
`docs/core.md:501-508` — both deleted). The boundary rules are
unchanged from `docs/core.md` §5 and are already enforced by
construction + `no_inner_jumps`: Rust never `longjmp`s; only
`extern "C"` wrappers convert `Err`→jump; every call into C that may
jump goes through `robigo_try_*` trampolines
(`crates/robigo-core/src/unwind.rs:14-80`); C→Rust callbacks
(`Rf_eval` etc.) return `Res<Sexp>` from the evaluator directly — no
host table, no token round-trip through a hashmap. `Return(v, env)`
targets by ENVSXP address, which is now also the identity C sees, so
a `return` issued from R code evaluated under C (`R_forceAndCall`)
resolves against the same stack GNU R's `findcontext` would walk.

#### 5. Strings

- `RString`/`RStr` (`value.rs:43-60` and its manual refcounted
  allocation, one of the `unsafe` sites at
  `docs/architecture.md:143-144`) are deleted. A string element is a
  CHARSXP `Sexp` (8 bytes in a STRSXP vs 16 today); `NA_character_`
  is the immortal `NA_STRING` compared by address. The CHARSXP cache,
  encoding bits and translate machinery already exist and are tested
  (`docs/core.md` §4, `crates/robigo-core/src/chars.rs`); runtime
  string builtins call them through `obj.rs` instead of the bridge's
  `char_sexp`/`rstr_of` conversions (`docs/core.md:404-411`).
- **`Sym` stays `u32`** (`crates/robigo-runtime/src/sym.rs:13-14`):
  predefined symbols remain compile-time constants (`s::NAMES` etc.),
  descriptor indices and frame masks keep working, and symbol
  comparison stays an integer compare in the VM. The isolate holds
  `symbols: Vec<Sexp>` (index → SYMSXP from `Core::install`,
  `gc.rs:895-906`) and the reverse map is the SYMSXP's PRINTNAME.
  Interning a new name is `cx.mk_char` + `install` + push — the only
  place the two tables meet. This is a deliberate non-churn decision:
  making `Sym = Sexp` would touch every `s::` site for zero
  measurable gain (symbols are immortal; the load is L1-hot).
- What builtins see: `cx.char_bytes(c) -> &[u8]` (borrow rule §1.3),
  `cx.char_str(c) -> Option<&str>` (Some iff ASCII bit set, or UTF8
  bit set and bytes valid — ASCII is the hot test, one `gp` load),
  and explicit translate points `cx.translate_utf8(c) -> Cow<[u8]>` /
  `cx.translate_native(c)` implementing `needsTranslation` exactly
  (`docs/core.md:382-394`). Text algorithms that are encoding-blind
  in GNU R under `useBytes`/C locale operate on bytes; the planned
  `robigo-text` split (`docs/architecture.md:58,214-220`) is
  unaffected since it already takes `&[u8]`.
- **Performance.** `paste`: build into a reused `Vec<u8>`, one
  `mk_char_len_ce` at the end (hash + open-addressing probe,
  `docs/core.md:366-369`). Risk: interning every long unique paste
  result costs a hash of the full bytes where today an `Rc<str>`
  alloc does not hash at all; GNU R pays the same, and pointer
  equality is load-bearing for packages (data.table,
  `docs/core.md:395-397`), so intern always — but the prototype
  measures `benchmarks/suite/strings.R` before/after (§10).
  `match`/`unique`/`duplicated` on character vectors hash the
  *address* (interning makes equal strings identical pointers; NA is
  one immortal address) with the bridge's multiplicative mix
  (`crates/robigo-capi/src/bridge.rs:31-53`) — strictly cheaper than
  today's content hashing, and the cached `char_hash` at +8
  (`crates/robigo-core/src/lib.rs:722-725`) serves the non-address
  tables. Sorting still compares bytes/collation as today.

#### 6. Vectors and ALTREP

Storage variants are exactly core's header-flag taxonomy
(`docs/core.md:150-161`), now the runtime's only taxonomy:

| Variant | Representation | Runtime access |
|---|---|---|
| Owned | inline payload at +32 | `RVec`/`RVecMut` slices (§1) |
| Compact seq | ALTREP, core's own intseq/realseq classes | `elt`/`region` fast paths inline the arithmetic (own classes called as Rust, `docs/core.md:479-481`); `RVal::Range` in registers (§7) materialises to this |
| Package ALTREP | ALTREP, class methods via trampolines when C, direct `Res` when ours | `cx.elt_*`, `cx.*_region`, `cx.dataptr_ro/rw`; `Length` never forces elements |
| Foreign (Arrow/mmap) | ALTREP class per `docs/core.md:156-161,178-228` | read views through `Dataptr_or_null`; writable access materialises once |
| EXTDATA | bridge-only today | **deleted for runtime vectors** (no second buffer to alias); kept in core for capi's own uses |

Deleted with this unification: the runtime compact registries
(`value.rs:503-540`), the package-ALTREP copy registry and deferred
wrappers (`crates/robigo-runtime/src/altrep.rs`,
`docs/core.md:748-807` — including the `may_stay_deferred` forcing
site list, since nothing is deferred-by-copy any more: a builtin
receives the ALTREP object itself and reads it through the same
accessors GNU R's C does, which is the "exact per-operation method
calls" that `docs/core.md:803-804` defers to G3). JIT kernels refuse
ALTREP inputs at guard time (flags test) rather than by registry
lookup — one byte test replaces today's `any()` + weak-count dance
(`altrep.rs:44-55`).

**Growth.** `x[n+1] <- v` amortised growth uses core's
`length`/`truelength` split: add `Core::alloc_vector_cap(t, len, cap)`
(size class from `cap`, `truelength = cap`, `gp::GROWABLE`);
`cx.grow_for_append(x)` in subassign doubles capacity (geometric,
GNU R's `growVector`) only when `!MAYBE_SHARED && owned && len<cap`
fails. `SETLENGTH` within capacity is then a header store, matching
GNU R including the shortened-vector end-pointer rule
(`docs/ledger/core.md:37-43`). Attributes go through one
implementation: runtime `attrib.rs` and capi `attrib.rs` merge (the
pairlist is now shared truth, `setAttrib` validation written once),
with `get_attrib`'s hot path (names/dim/class) as `&Cx` reads.

Serialized/compact row names, `1:2e8` in constant memory (G3 exit
gate, `docs/design.md:569`) fall out: `:` returns the compact ALTREP
object itself everywhere, not a register-only range that materialises
on binding (today's split at `vm.rs:47-67`).

#### 7. VM and JIT on Sexp

**Register file.** `RVal` (`crates/robigo-runtime/src/vm.rs:41-54`)
keeps its enum shape with `V(Value)` now 8 bytes; I do *not* propose
NaN-boxing or pointer-tagging registers — `NA_real_` is a NaN payload
we must preserve bit-exactly, and tagged-pointer tricks would fight
the C ABI at every materialisation. The register file of each live
activation is one registered root range (§2.2): registers live in an
isolate-owned arena of `RVal` slots (grown, recycled like today's
stack `SmallVec`s but at a GC-known address), and the marker visits
only slots whose kind is `V`. Two implementation options for the
marker to know kinds: (a) read the enum discriminant (requires a
stable `#[repr]` layout for `RVal` — set `#[repr(u8)]`-style explicit
tag word), (b) parallel kind bytes. I choose (a) with an explicit
layout: `struct RVal { tag: u8, .. }` union-free via a 16-byte
`{bits: u64, aux: u64, tag}`… — in practice: keep the Rust enum for
type safety in the VM loop, add `#[repr(C)]` mirror + `const`
assertions on offsets in `roots.rs`, and have the marker read the
discriminant through the mirror. *(If repr guarantees prove brittle
across rustc versions, fall back to (b); the prototype decides, §10.)*
Dead registers hold a scalar tag (today's #357 discipline,
`vm.rs:842-847`) so GC never retains through them — now a correctness
property the torture suite checks, not a comment. `Vars` slot caches
flush at exactly today's points (`vm.rs:1-15` contract unchanged);
flushing now also boxes pending scalar slot values into bindings
(§4.2) — one write per dirty slot, replacing `set_var`'s `Rc` tricks
(`docs/ledger/perf-vm.md:2078-2081`: redo, don't port).

**Carry-over** from `docs/ledger/perf-vm.md:2054-2077`, mapped:
call-site function lookup (`call_site_fun_fast` → descriptor/global-cache
probe §3.3); pure-closure inlining (unchanged logic, `RVal` in/out);
caller-forced promises (§4.2); index promises forced in R's order
(unchanged); guard memoisation (keys now read `fn_version` at env+48
— one load, no `RefCell` borrow, strictly cheaper than
`env.rs:901-906`); static rejection memos (in `Code`, unchanged);
`%*%` row-vector dot products (in builtins, on `RVec<f64>`).
Redo-don't-port per the ledger: `SmallVec` register buffers (→ arena
above), `set_var`/`own_var` (→ slot store + `ensure_unique`), cold-path
placement in `run_with` (re-measure codegen; keep the dispatch loop
small from day one, with `Index`/call general paths `#[cold]` as the
ledger's lesson at `docs/ledger/perf-vm.md:2190-2195` prescribes).

**Inline caches.** `CallSite` (`compile.rs:636-692`) keeps its fields
with `memo: (search_epoch, env_addr, Value)`; `GuardMemo` shrinks to
`(epoch u64, env addr, fn_version u32, ok)` since versions are header
loads. Add one new cache the new layout makes cheap: **slot hints**
per `GetVar` site — `(env_addr, bind_version, slot_index)` —
the "verified slot hints" the #523 entry lists as next work
(`docs/ledger/perf-vm.md:80-81`: `plain_lookup` ~92 instr/lookup,
645 of 5,794 in the profiled general call). In slot-mode frames a
hint hit is two compares + one `VECTOR_ELT`; this is the single
biggest expected win on `calls.R` and the prototype's headline
measurement.

**JIT.** Cranelift tiers (`jit.rs` loops, `jit/closure.rs` call trees)
keep their structure, deopt contracts and `JType` lattice; what
changes is the object ABI they compile against, now *stable constants
from core* instead of `Rc` layouts: type +0, flags +1, gp +2,
refcnt +4, attrib +8, length +16, data +32
(`crates/robigo-core/src/lib.rs:155-182`), env fn_version +48, slot
vector at env+32. Guards become byte tests (`flags & (OBJECT|ALTREP|
EXTDATA) == 0 && typ == REALSXP`) then direct payload access —
replacing today's `VecView` extraction through `Rc<Vector<T>>`
(`jit.rs:121-134`) with `(data_ptr, len)` loaded from the header,
revalidated after every helper call (helpers are safepoints §1.3).
`JType::Dyn` slots (`docs/ledger/perf-vm.md:194-202`) become `Sexp`
slots in a per-activation Dyn frame array registered with the GC
(Julia-style frame, §2); native code may hold a `Sexp` in a machine
register only between safepoints, and spills live Dyn values to the
frame at every helper call — the existing `deopt_regs`/`RegState`
machinery (`jit.rs:192,1617-1707`) already computes live sets per pc,
so safepoint spills reuse it. Allocation from JIT code goes through
a helper (`cx` in a pinned register) — no inlined allocator until
measured otherwise. Deopt is unchanged in spirit: reconstruct `RVal`
registers, VM executes the op, warnings/errors are the VM's
(`docs/architecture.md:170-175`).

#### 8. The isolate record

One struct, one TLS pointer, `&mut Cx` threaded explicitly:

```rust
// crates/robigo-runtime/src/isolate.rs
pub struct Isolate {
    pub core: Box<Core>,                 // heap, CHARSXP cache, symbols' Sexps
    pub interp: InterpCore,              // frames, handlers/restarts, options, rng, conns, dlls,
                                         // fun/global cache, epochs, arg/promarg pools (interp.rs:79-252)
    pub roots: Vec<Sexp>,                // §2.1
    pub ppstack: ...,                    // moved from capi state (R_PPStack array + top)
    pub symbols: Vec<Sexp>,              // Sym(u32) -> SYMSXP (§5)
    pub capi: CapiTables,                // today's state::Tables (crates/robigo-capi/src/state.rs:27-53)
    pub vm: VmState,                     // register arena, guard epochs, JIT module/code cache, retire list
    pub text: LocaleState,               // locale.rs + core chars locale mirrors
    pub pending: Option<Box<Flow>>,      // §4.4
}
pub type Cx = Isolate;
thread_local! { static ISOLATE: Cell<*mut Isolate> }  // THE one; capi's state.rs:91-93 pattern
```

What moves in: all 59 runtime `thread_local!`s and capi's remaining
ones (`docs/ledger/core.md:252-257` inventories them: `bridge.rs`'s
die with the bridge; `errors.rs` PENDING/SCRATCH, `context.rs`
CONTEXTS, `util.rs`, `envir.rs` buffers become fields; runtime's
epochs (§3.3), symbol interner tables (`sym.rs`), compact/altrep
registries (deleted §6), VM guard cells, JIT caches, `autogc`/
reachability cycle bookkeeping (deleted — the tracing heap collects
cycles, `docs/design.md:153-156` closed at last), format/scan/grep
scratch buffers, tz caches, connection state). Truly process-global
immutables stay out: the builtin `DEFS` tables, predefined symbol
*names*, NA bit patterns.

**Access discipline.** Hot paths receive `&mut Cx` (builtins, VM,
evaluator — they already receive `&mut Interp`) or downgrade to
`&Cx` for views. The TLS pointer is read only at: capi `extern "C"`
entry (one read per export call, as `state::st()` does today), GC
root registration (once at isolate creation), signal/interrupt
polling points that have no `Cx` in hand, and embedder entry. Never
in a loop. This directly removes the libR.so blockers measured at
`docs/ledger/core.md:202-212` (535,920 `__tls_get_addr` calls at
startup from runtime thread-locals): after G3 the revisit
(`docs/core.md` §12 item 15) is a re-measurement of the allocator-shim
term only. Exported per-isolate C globals (`R_GlobalEnv` etc.) remain
process-wide variables *written from the isolate* for the one-isolate
executable shape, with robigo's headers macro-ising them over the
isolate accessor for the multi-isolate future, exactly as
`docs/core.md:984-998` prescribes. I do **not** promise multi-isolate
concurrency in G3 — one R thread per isolate, `collect()` already
aborts off-thread (`docs/core.md` §12 item 7) — G3 only stops
*adding* obstacles to it.

#### 9. Migration strategy

##### 9.1 Order of work

**Phase 0 — prototype (§10).** Throwaway, in `experiments/g3proto`.
Gates everything below.

**Phase 1 — layout-independent prep on main** (each step ratchet-green
and perf-gated on its own, mergeable in any order):
1. Isolate record for *state only*: move thread-locals into fields of
   `Interp`/a new `Isolate` that owns `Interp` + capi `State`, keeping
   `Value` untouched. Mechanical, per-file PRs following the pattern
   already proven free by the capi table move
   (`docs/ledger/core.md:237-251`). Exit: one runtime TLS pointer
   remains (plus capi's ISOLATE aliasing the same record).
2. Core changes A+B (§3.1, §1.4) + barrel of offsets for JIT: land in
   `robigo-core` behind the existing capi (ENVSXP from capi becomes
   80 B general-mode; FRAME/HASHTAB shims adapt). No runtime change.
   Exit: capi corpus unchanged, core tests + torture green.
3. `obj.rs` written and unit-tested against core *without* converting
   builtins (it is dead code on main except its tests): `Value`
   wrapper (under a temporary name `Obj` to avoid collision), views,
   builders, root scopes, debug-epoch checks. Exit: council item 24
   has a spec that compiles.
4. Census + codemod dry run (§9.2) on a scratch branch, to size
   phase 2 honestly before scheduling it.

**Phase 2 — the swap** (short-lived branch, target ≤7 days wall):
rename `Obj`→`Value`, delete `value.rs`, convert in dependency order
with the tree *compiling at every commit* via a temporary compat
layer in `obj.rs` (old-shaped accessor functions marked
`#[deprecated]`, deleted in the last commit): (a) `env.rs`,
`interp.rs`, `conditions.rs`, `dispatch.rs`; (b) `compile.rs`,
`vm.rs`, `jit*` (JIT initially *disabled* behind its existing env
switch, re-enabled as its own step once VM numbers land); (c)
builtins module by module (§9.2); (d) `print/deparse/format/coerce/
attrib/serialize`; (e) capi host-table deletion + bridge deletion
(§9.4) — deliberately *last in the same branch*, because until then
the bridge can still run (converting `Value`==`Sexp` becomes the
identity function progressively; HOST proxies stop being created as
env/closure/promise become real nodes in step (a), and the proxy code
paths go dead before they are deleted). Ratchets: run per-module
conformance slices + fastr slices per commit batch in CI on the
branch; the full gate (conformance all tiers, corpora, capi, package
smoke, perf compare) must be green *before* merge, with ratchet
marker updates only ever adding passes.

**Phase 3 — on main after merge:** re-enable/tune JIT (§7), slot
hints, sticky-mark generational evaluation (`docs/core.md` §11) if
pauses demand it, libR.so revisit measurement (§8), `robigo-text`
split.

##### 9.2 Converting ~60k lines of builtins

Three classes, sized by the Phase-1 census (fractions are *(guesses)*
until then): **(i) mechanical (~55%)** — constructor renames
(+`cx`), `match v { Value::Real(r) => &r.data }` →
`match v.view(cx) { View::Real(xs) => &*xs }`, `.len()`,
`same_object`→address eq. A syntax-aware codemod (rust-analyzer/
`syn`-based, checked into `scripts/g3codemod/` for reviewability)
does these; every codemod rule ships with before/after fixtures.
**(ii) semi-mechanical (~30%)** — `Rc::make_mut` sites (fresh vs
shared distinction §1.4), attribute access through `AttrBox` →
pairlist helpers, `RString` handling → `char_bytes`/`translate_*`,
pairlist/lang `Vec<Arg>` walks → cons walks (with a `cx.pairlist_vec`
escape hatch for cold code). Codemod proposes, human confirms per
module; each builtin module maps to a GNU R C file, so conversion is
also a re-port check against the C — the porting contract
(`AGENTS.md:45-49`) makes this a feature, not overhead.
**(iii) hand (~15%)** — subassign/subscript in-place windows,
serialize, print/deparse, envir/lang reflection, dotcode.
One writer per file (`docs/design.md:97-98`), modules pre-registered
in a conversion ledger with their conformance slice as the done-test.

##### 9.3 The mixed-heap question, and the new-crate alternative

The coordinator is right that a staged mixed Rc/GC heap is unsolved,
and the repo already contains the proof: one mark over two heaps
needed holds, watches, twins, `mark_proxies`/`trace_proxy` and a
stated invariant that every ignored runtime edge be a core edge
(`docs/core.md:712-746`) — and *still* leaks cross-heap cycles by
admission (`crates/robigo-capi/src/bridge.rs:8-13`). Attributes point
anywhere, so there is no seam along which "vectors first, envs
later" can be cut without re-inventing twins inside the runtime.
Rejected as a steady state; tolerated only as the compile-preserving
compat layer *inside* the swap branch (same heap, old-shaped
function names — a typing shim, not a second heap).

A new runtime crate grown beside the old one (`robigo-runtime2`)
fails differently: builtins, evaluator and VM are typed on `Value`
throughout, so "sharing" them means copying them — the same
conversion work, plus an unbounded period in which two evaluators
must be kept semantically identical against a moving main (the Rho
failure mode, `docs/design.md:42`), plus a second swap (crate
replacement) at the end. The only parts genuinely sharable
(syntax, numerics, text) already are separate crates. Rejected —
except as the throwaway prototype, where being beside and incomplete
is the point.

##### 9.4 When the bridge dies, and what pauses

Bridge deletion is Phase-2 step (e), deleting in order: twins +
SEEN/STORED/PINNED/watch machinery (dead once identity is native),
HOST proxies + `host.rs` table (dead once env/clo/prom are nodes),
EXTDATA zero-copy argument path (dead once vectors are core),
`sync_stale_promise`/write-backs/retire lists, then `bridge.rs`
itself; `.Call` becomes: build the argument pairlist (already Sexps),
protect, trampoline, return the Sexp. Each open two-heap issue dies
with a named mechanism: #278/#304 (address identity — one object),
###277 (one PROMSXP §4.2), #276 (C writes the binding's own box;
`binding_object` §4.2), #189 (env_clone clones one ENVSXP), #167/#389
(no copies to free), #217 (no write-back pass to free under the
evaluator), #460 (compact ALTREP everywhere §6), #140 (tracemem
reads the header `extra` trace bit + `ensure_unique` reports §1.4).

Pause on main during Phase 2 only (≤7 days): runtime-touching merges
except conformance *test additions* and core/capi work that does not
touch `Value`. Before Phase 2, per `docs/design.md:587-589`, new
builtins/stats ports already wait; work that continues throughout:
syntax, numerics, capi census coverage, perf-vm techniques that
carry over (§7), package ratchets, and — importantly — more
conformance coverage of the general call path, which de-risks the
swap more than any prep refactor.

#### 10. Prototype and measurement plan

`experiments/g3proto/` (own workspace, like `experiments/memory`):
a minimal evaluator *and* a minimal register VM on `robigo-core`,
reusing `robigo-syntax` for parsing and building LANGSXP directly.
Scope: closures + argument matching (positional/exact/defaults),
slot-mode and pairlist-mode envs (runtime flag), promises (heap +
caller-forced), scalar arithmetic/comparison, `[[`/`[` on lists and
atomic vectors, `c`/`list`, `<<-`, `for`/`while`/`if`, enough of
`calls.R` minus S3/tryCatch — then S3 dispatch and tryCatch as the
stretch week, since they stress contexts. Both rooting modes behind
a flag (precise scopes vs whole-stack conservative) to settle §2 by
numbers, not taste. Both `RVal` marker options (§7) behind a flag.

Decisions → measurements → exit criteria (all with
`scripts/perf.py`-style callgrind instruction counts, same-binary
comparisons per `docs/ledger/perf-vm.md:73-75`; wall time secondary
on this shared machine):

| Decision | Workload | Metric | Exit bar *(proposed)* |
|---|---|---|---|
| Rooting: precise vs conservative-Rust | fib, binarytrees, calls-subset; finalizer stress (core ledger's depth script, `docs/ledger/core.md:14-18`) | Ir/call; finalizers missed in 100 runs; torture clean | precise within +5% Ir of conservative **and** 0 missed finalizers (conservative expected to fail the second bar — that failure *is* the decision) |
| Slot vs pairlist frames; 80 B env | fib, calls-subset, `benchmarks/suite/closures.R` | Ir/call, allocs/call, bytes/call | slot ≤ today's VM Ir/call (5,597 for the profiled general call, `docs/ledger/perf-vm.md:67`) and ≤ pairlist −15% |
| Caller-forced + unboxed promise path | fib, `scale_by`-shape calls | allocs/call (heap counter) | ≤ today (prototype reports vs VM's ~0.003–2 allocs/call range, `docs/ledger/perf-vm.md:19-20,52-61`) |
| Allocation fast path vs `Rc` | binarytrees | Ir, peak RSS, GC pause p50/p99 | RSS ≤ GNU R; no pause >10 ms at this scale |
| String interning (`paste`) | `benchmarks/suite/strings.R` slice | Ir | within +5% of today's runtime slice, else §5 gets a no-intern fast path for long strings (with pointer-eq consequences re-councilled) |
| Borrow discipline ergonomics | the prototype's own builtin set (~15 builtins) | count of `revalidate` sites; safepoint-during-view compile errors caught | every violation is a compile error, not a review comment; revalidate sites counted and reported, no bar |

The rebuild starts when: precise rooting passes torture + finalizer
bars, slot frames beat the Ir bar, and the codemod census (§9.1
Phase 1.4) shows ≥50% mechanical sites. If slot frames miss their
bar, G3 still proceeds (one heap is worth it alone) with pairlist
frames and slot hints deferred — that fallback is decided *here*, in
advance, so a miss is data rather than a schedule crisis.

#### 11. Risks and open questions, ranked

1. **Precise rooting is a discipline, and 60k lines of builtins were
   written under `Rc` (forgetting is impossible there).** A missed
   root is a use-after-free that only `gctorture` catches.
   *Resolution:* torture runs of the full conformance + capi suites
   in CI on the swap branch (core/capi already run under it,
   `docs/core.md:300-301`); debug-epoch views (§1.3); the §10
   ergonomic count. Residual risk stays highest — budget the most
   review here.
2. **GC pause/memory on churn at package scale.** The bake-off's
   binarytrees weak spot (61 MB, 15–40 ms marks,
   `docs/ledger/memory.md:87-88`) meets real package suites already
   at 0.7–1.0× GNU R. *Resolution:* prototype pause/RSS bars (§10);
   if missed, sticky-mark generational collection behind the
   existing `store` barrier (`docs/core.md` §11) becomes a Phase-3
   fast follow, with `flags::SPARE` marking DATAPTR-exposed objects
   always-dirty (`crates/robigo-core/src/lib.rs:90-94`).
3. **`&mut Cx` split ripples further than `Value` itself.**
   Accessors that today take `&Interp` or `&Value` and secretly
   allocate/warn will surface as type errors mid-swap. *Resolution:*
   the Phase-1 census must classify safepoint-ness of the top ~100
   helpers (`coerce::*`, `attrib::*`, `as_*`) before Phase 2 starts.
4. **REFCNT imprecision (dead-parent counts never decremented,
   `docs/core.md:350-352`) may duplicate more than `Rc` did** on
   real code (e.g. growing a vector in a loop after its parent list
   died). *Resolution:* conformance `tracemem` scripts + a
   copy-counter instrumented run of `dataframe`/`calls` comparing
   duplicate counts old vs new; if bad, a count-repair pass during
   sweep is possible (sweeper knows dead parents) at measurable cost.
5. **ENVSXP at 80 B + slot VECSXP = 2 allocations per call frame**
   vs today's 1 (`env.rs:63-67` inline bindings). *Resolution:* §10
   bars; mitigation ready if needed: small-slot frames inline the
   first N slot values in a larger env allocation (core size-class
   variant of `alloc_env`) — designed only if measured.
6. **JIT Dyn safepoint spills** may cost more than today's
   reference-counted slots on `binarytrees`-shaped code.
   *Resolution:* re-enable JIT as its own gated step (§9.1) with
   `jit.rs` by-function callgrind diffs; worst case the loop JIT
   (scalar-only) ships first and closure trees follow.
7. **Lazy RCNTXT publication** (§4.4) assumes no package caches
   context pointers. *Resolution:* grep the G4 package set +
   methods.so imports before Phase 2; the eager fallback is a
   contained change in capi's context module.
8. **Codemod silent mis-conversions** (e.g. a `match` arm whose
   fallthrough relied on enum exhaustiveness). *Resolution:*
   `View` is `#[non_exhaustive]`-free and matching stays exhaustive
   by construction; codemod fixtures (§9.2); the interpreter-vs-VM
   byte-identity gate (`compiled_matches_interpreted`) runs per
   module batch.
9. **`Sym` as index vs SYMSXP identity in serialization/srcrefs:**
   symbol *order* in the base env listing is GNU R's global symbol
   table order (`env.rs:90-92`); the isolate `symbols` vector's
   order must match interning order from bootstrap. *Resolution:*
   existing conformance scripts for `ls(base)` ordering; verify in
   Phase 1 since it constrains bootstrap sequencing.
10. **Fork/CoW behaviour of the new env header writes**
    (`bind_version` bumps dirty env pages in forked children where
    side mark bitmaps avoided it, `docs/core.md:961-975`).
    *Resolution:* the §10-of-core.md `mclapply` RSS measurement
    (`docs/core.md:1011-1013`) re-run post-G3; versions are mutator
    writes like refcounts, which GNU R also makes — expected
    acceptable, currently *(guess)*.

*End of design. Deliverable: this file (`muse.md`).*



### Synthesis

### Round 1 synthesis (coordinator)

Seats: GPT-6 Astra, Claude Fable 5.2, Claude Opus 5.5, Muse Wonder Full.
(GPT-6.2 Sol was requested but not available.) Base `4cff0ceb`.

#### Coordinator's checks of load-bearing claims (all held)

- The tracer marks car/cdr/tag/aux of LISTSXP, LANGSXP, DOTSXP, CLOSXP,
  ENVSXP, PROMSXP, BCODESXP, OBJSXP and SYMSXP unconditionally
  (`crates/robigo-core/src/gc.rs:1349-1354`): any unboxed scalar in a node
  word needs a tracer change.
- Weak references and the finalize queue are processed inside
  `collect_needing` (`gc.rs:1524-1577`) after marking: Fable's reorder
  (conservative words marked after weak processing) is a local change.
- GNU R defers link counts for byte-code stack values: `eval.c:997-1016`
  (`R_BCProtCommitted`, `INCLNK_stack_commit`). Typed binding cells:
  `Defn.h:462` (`BNDCELL_TAG`).

#### Unanimous

1. One heap; no mixed Rc/GC heap at any stage (the bridge-identity bugs
   #278 #277 #276 #189 #304 #167/#389 #217 are persistent cross-heap
   references).
2. "The swap is local to `value.rs`" is false (ownership, strings,
   `Vec<Arg>` calls, `RefCell` frames, the in-place protocol leak).
3. Prototype first, measured on the general call (`calls.R`'s six idioms),
   not only fib, with numeric exit criteria.
4. Explicit `gc()` must not treat stale Rust-frame words as roots
   (finalizer determinism, core ledger 2026-10-28 and 2026-11-11).
5. GNU R's REFCNT exactly. Opus and Fable add GNU R's deferred link counts
   for stack values; Muse omitted them (a gap: `c(x, {x[1] <- 0; x})`).
6. Closure frames: index-addressed slot storage, one-way conversion to GNU R
   binding cells on C/reflection exposure; hashed/user envs as GNU R.
7. Strings are CHARSXPs, NA immortal, pointer hashing in match/unique when
   encodings agree; interning cost of `paste` measured.
8. Tagged register slots as precise roots; no NaN-boxing; JIT spills objects
   to slots before helpers (no stack maps); header offsets from core
   constants; compact sequences real ALTREP objects everywhere.
9. One TLS read at C entry; `&mut Cx` passed down.
10. Pause layout-tied VM tuning, new bridge mechanisms, RString work.

#### Disputes

1. **Handle**: Astra scope-branded `Value<'s>`; Opus `Ref<'g>` branded by
   `&Cx` (alloc needs `&Cx`, R/C entry `&mut Cx`; needs deferred warnings in
   pure code); Muse plain `Copy` `Value` with views borrowing `&Cx` and
   safepoints taking `&mut Cx`; Fable plain `Copy` `Value`, `keep()` roots,
   runtime-checked strict mode.
2. **Rooting**: Astra/Muse precise Rust roots, conservative only in C
   segments; Opus conservative scan of the innermost Rust segment only
   (explicit gc scans no Rust frames; allocation-triggered collections stay
   nondeterministic); Fable precise roots decide reachability/finalizers and
   conservative words only pin memory afterwards (risk: a missing root lets
   a finalizer run on an object Rust still holds — an EXTPTR finalizer then
   frees C resources in use). Fable's "experiment A" can run on main today.
3. **Unboxed promise/binding values**: Muse registers only; Astra measured
   variant; Opus in the node's 4th word with a gp tag; Fable typed binding
   slots.
4. **Migration**: Astra prototype crate on main then one short coordinated
   branch; Muse API as dead code on main then a ≤7-day swap branch; Opus a
   new `robigo-rt` crate on main with its own ratchet column, re-port from
   C, flip and delete (boot-cliff risk, divergence risk); Fable one API over
   two backends on main (builtins converted under today's ratchets over Rc,
   new evaluator behind a `g3` feature with its own ratchet).

## Round 2

### Brief

### Council round 2: redesign G3 under the lead's framing

Same council, same repository and rules as round 1
(`brief.md` in this directory: read it again for the repo path, the read-only
rules, the state measured at `4cff0ceb` and the eleven parts to design).
Read-only on the repo; no subagents; no builds that write into the repo.

#### The lead's framing (new, binding)

Max, the lead, said after reading round 1:

> We have no users who need support or backwards compatibility. We can rebuild
> anything that gives us a benefit and treat effort as free. Be bold.

So: **the end state wins.** Choose the design that makes robigo the fastest
exact R, with the strongest guarantees, regardless of how much code it
rewrites. Migration risk still matters only where it threatens *correctness*
or *the ability to verify* (ratchets, oracle, measurements); it does not
matter as effort, churn or calendar time. Nothing is sacred except the
contract: GNU R's observable behaviour byte for byte (output, messages,
warnings, errors, status), source compatibility with GNU R's C API for
packages, and measurement as the arbiter. That includes `robigo-core` itself:
its object layout, header, node size, collector, CHARSXP cache, unwind design
and the bake-off's decision are all open if you can argue a better end state
(say how it would be measured). Also open: the VM's architecture, the
compiler, the AST shape, the JIT tiers, crate boundaries, the porting rules in
`AGENTS.md`.

#### Inputs for this round

- All four round-1 designs, in this directory: `astra.md`, `fable.md`,
  `opus.md`, `muse.md`. Read all four, not just your own. Steal freely.
- `synthesis-r1.md` (this directory): the coordinator's synthesis — where you
  agreed, the four disputes, and the coordinator's checks of your claims.
- **The target moves to R 4.6.2** (lead's decision pending, assume it). The
  4.6.2 sources are unpacked at
  `<R-4.6.2 sources>`
  (4.5.0 stays at `~/.cache/robigo-research/rsrc/R-4.5.0`). Read
  `doc/NEWS` lines 593-708 ("C-Level Facilities" of 4.6.0): R 4.6 hides
  `FRAME`, `HASHTAB`, `ENCLOS`, `ENVFLAGS`, `TRUELENGTH`, `LEVELS`, `NAMED`,
  `OBJECT`, `SETLENGTH`, `STDVEC_DATAPTR`, `SET_BODY/CLOENV/FORMALS`, the
  `EXTPTR_*` accessors from installed headers; removes `VECTOR_PTR`; warns
  on `ATTRIB/SET_ATTRIB`, `BODY/FORMALS/CLOENV`, `PRVALUE/PRCODE/PRENV`,
  `Rf_allocSExp`, `STRING_PTR`; and adds binding/dots APIs
  (`R_GetBindingType`, `R_MakeDelayedBinding`, `R_DotsElt`, ...),
  `R_getVar`, `R_mapAttrib`, `DATAPTR_RW`. Use what this frees up.
  Base-package natives are still built from GNU R's sources against
  robigo's own `Defn.h`, so they constrain nothing package-visible.

#### The coordinator's tentative picks under the new framing (challenge them)

1. Migration: Opus's plan B — a new `robigo-rt` crate on main with its own
   ratchet column, builtins re-ported from GNU R's C onto an API designed
   only for the target (not constrained to also run over today's `Rc`
   model), flip when its ratchets contain the old ones and no benchmark is
   slower, delete the old runtime and the bridge in the flip.
2. Handle: compile-time branding (`Ref<'g>` tied to `&Cx`; anything that
   runs R or C code takes `&mut Cx`), the strongest static guarantee.
3. No deferred warnings: every warning signalled immediately, as GNU R.
4. Every layout/representation variant (frame layout, unboxed promise
   values, binding cells, register-slot layout, rooting scheme) is built in
   the prototype and the fastest exact one wins.

#### Deliverable

Write a complete revised design to the file named in your task message. It
must stand alone (a reader should not need your round-1 file), and start with:

1. **What changed from your round 1, and why** (a short list).
2. **Your position on each of the four disputes** in `synthesis-r1.md`, and on
   each of the coordinator's four tentative picks.
3. **The boldest changes you now propose** that round 1 held back on because
   of effort — anywhere in the stack — each with the end-state benefit, the
   measurement that would prove it, and what it deletes.
4. **The best idea from each of the other three designs** that you adopted,
   and the one you think is wrong, with the reason.

Then the full design (the eleven parts of `brief.md`), concrete: types,
offsets, file names, GNU R `file.c:function` references, measurements and
exit criteria. Ground claims in code you read (`file:line`); mark guesses.
Your final chat reply: the file path and a five-line summary.


### Astra

### G3 revised design: one object graph, explicit effects, exact observation

#### What changed from round 1, and why

- Adopt a new `robigo-rt` crate on main, with independent conformance columns, followed by one default flip and deletion. My coordinated conversion branch bought no end-state benefit. Independent executable verification is the useful boundary.
- Make allocation an exclusive execution operation. `Ref<'g>` borrows `&Cx`; every operation that may collect, mutate, signal, or call external code requires `&mut Cx`. Precise scoped roots survive those operations. This combines the useful parts of Opus's branding and my precise rooting without relying on LLVM's treatment of unrooted pointers.
- Replace mandatory environment promotion with native slot frames throughout their lifetime. R 4.6 hides FRAME, HASHTAB and ENCLOS. Public binding APIs operate on the canonical slots. Only an actual exposed cell requires cell-backed storage; merely calling C or inspecting a binding does not.
- Make typed promise values and virtual promise records first-class candidates, with a complete observation contract. The tracer is ours to change. Preserving its current pointer-only arm is no reason to allocate scalar boxes.
- Rebuild compilation around an effect-aware intermediate representation and precise resume states. General closures, including calls that signal or mutate, should eventually benefit from native code. Preserve the existing pure-call fast path as an independently verified specialization.
- Reopen node layout, allocator and collector policy against the complete evaluator. Keep non-moving published objects, but make no promise to keep 48-byte cons cells, a universal fourth word, or the current collection schedule.

#### Positions on the disputes and tentative picks

| Round-1 dispute | Revised position |
|---|---|
| Handle | Branded short references plus scoped precise roots. A bare `Copy Sexp` is private to audited low-level code. The `&Cx` brand proves a read interval, not reachability across allocations. Allocation takes `&mut Cx`; an argument retained across it must already have a root or be passed through a rooted input window. |
| Rooting | Precise Rust, VM and JIT roots. Retain the existing bounded conservative C segments initially. Reject conservative Rust segments as a soundness foundation, and reject finalizer classification before pinning as a safety net. A finalizer can free an external resource even if the SEXP's allocation stays pinned. |
| Unboxed promise and binding values | Implement and measure typed canonical bindings and promises, together with boxed controls. Also measure virtual promise records that materialize once. Never interpret scalar bits as pointers; trace by allocation kind and payload tag. |
| Migration | Opus plan B, without a reverse bridge. Keep old and new runtimes in separate executables. Re-port semantics from the selected GNU R source while reusing proven pure kernels. No dual-backend API requirement. |

| Coordinator pick | Position |
|---|---|
| New runtime on main, own ratchet, flip and delete | Agree. Amend the superset gate for the R version change: preserve test identities, but compare new behavior with recorded 4.6.2 output. A legitimate oracle change is not a failure to reproduce 4.5.0. |
| `Ref<'g>` tied to `&Cx`, R/C entry takes `&mut Cx` | Agree only with the allocation and mutation rule above. Opus's shared-borrow allocator depends on conservative retention, which a lifetime does not establish. Reject that part. |
| Every warning signalled immediately | Agree. This includes calling handlers and restart establishment. GNU R may defer printing a warning; robigo must reproduce that separately. Errors also run calling handlers before unwinding the issuing logical context. |
| Build every layout variant and choose the fastest exact one | Agree for a finite, stated set of sound variants, including interactions. Reject benchmarking unsound candidates as possible winners and the implication that one scalar metric always selects a winner. Use correctness first, then per-workload speed, memory and latency constraints. |

#### Bold changes, their benefit, proof, and deletions

| Change | End-state benefit | Measurement that earns it | What it deletes |
|---|---|---|---|
| Canonical typed slots for all ordinary environments | Direct indexed variable access and no evaluator-to-environment writeback | All six `calls.R` idioms, escaped-frame and active-binding tests, allocation count and lookup instructions | `Vars::flush`, Rc frame borrowing, routine slot-to-pairlist promotion |
| Effect-aware compiler shared by VM and native tiers | Optimize general calls while preserving observation order | Package hot closures, S3 and tryCatch workloads, compile time, exact forced-deopt runs | Duplicated tier analyses, separate ad hoc eager-call rules, broad replay outside proven pure regions |
| Virtual activations and promises with exact materialization recipes | Remove frame, promise and context allocations where nobody can observe them | Allocations per call, `bindingType`, `missing`, `substitute`, stack introspection, C observers at every transition | Eager per-call cons lists and environment allocations in proven cases |
| Per-kind object layout and non-moving young collection candidate | Better locality and shorter collections under call churn | Updated seven-workload bake-off plus package suites, RSS, cache misses, p99 pauses, fork dirty pages | Universal node aux field if it loses; full-heap tracing on every young allocation threshold if generational policy wins |
| One fallible semantic implementation below C exports | Eliminate runtime/C-API drift and redundant work | C-to-R-to-C probes, export census, zero conversion counters, package timing | Bridge, HOST objects, twins, dirty synchronization, duplicate attrib/coerce/environment semantics |
| Compiler-derived roots and explicit effect boundaries | Enforce precise liveness while avoiding per-read handle pushes | Compile-fail API cases, forced GC at every safepoint, root stores and scanning time | Rust conservative scanning, manually inferred JIT pointer liveness, persistent unsafe builtin payload access |

These are proposed changes, not measured gains. They are conditional on the experiments below. In particular, a smaller node does not automatically beat the current size classes, and an SSA compiler does not automatically beat a compact VM on cold code.

#### Ideas adopted and rejected from the other designs

- **Opus:** adopt independently verifiable `robigo-rt` development and weak inline-cache references. Reject shared-borrow allocation backed by conservative scanning. `Vec<Ref>` bans cannot establish soundness for arbitrary safe containers or compiler-generated derived pointers. Also reject deferred warning signalling, even in apparently pure arithmetic: a calling handler can inspect and mutate the suspended computation's environment.
- **Fable:** adopt canonical environment slots and GNU R's deferred stack link counts. Reject the claim that precise-finalization plus conservative pinning prevents memory corruption. An EXTPTR finalizer can release the buffer that a pinned object still names. Reject the further claim that a rooted STRSXP cannot change: C can call `SET_STRING_ELT` on that same object, so its address alone cannot validate an S3 cache.
- **Muse:** adopt an unpublished construction API and separate Rust and C protection storage. Reject using a plain unrooted `Copy Value` as a safe general interface. Views branded to `&Cx` do not make a pointer safe to dereference after it survived an untracked collection. Reject keeping a documented forced-default mismatch; 4.6's binding API makes that mismatch more directly observable.

#### Evidence and scope

This is a read-only design at repository commit `4cff0ceb142dfa4547a840b1bfa4e1fa4b7360f4`. I ran no builds or benchmarks. Counts and timings in the brief are coordinator measurements. All thresholds below are proposed. Repository paths are relative to the worktree root. `R/` below denotes the supplied source tree:

`<R-4.6.2 sources>`.

Assume 4.6.2 as requested; record its source hash, configured features, public headers and oracle binary before implementation. The NEWS text at `R/doc/NEWS:593` distinguishes removed/hidden interfaces from interfaces that remain with warnings. These are not interchangeable. BODY, FORMALS, CLOENV, ATTRIB and promise accessors still require compatibility. Base natives compile against robigo's own Defn.h and impose semantic obligations, not frozen offsets.

The baseline general-call profile is 5,597 instructions after the latest promise improvement, with meaningful costs in lookups, register flushing, frames and promises (`docs/ledger/perf-vm.md:40`). Its six idioms lose to GNU R by 1.10–1.83x. Optimize that complete path rather than treating scalar boxing as the central problem.

#### 1. Rust object API and copy-on-modify

##### Types and capability boundaries

Proposed files: `crates/robigo-rt/src/obj/{mod,view,build}.rs`, `roots.rs`, `cx.rs`; raw storage in `crates/robigo-core/src/{layout,object,trace}.rs`.

```rust
// Schematic interfaces. Constructors and raw fields remain private.
###[repr(transparent)]
###[derive(Clone, Copy)]
pub struct Ref<'g> {
    ptr: NonNull<u8>,
    brand: PhantomData<&'g Cx>,
}

pub struct Root<'s> { slot: RootSlotId, scope: PhantomData<&'s ScopeBrand> }
pub struct Out<'s> { slot: RootSlotId, scope: PhantomData<&'s mut ScopeBrand> }
pub struct Scope<'s> { /* stable root slabs, isolate identity, lifetime brand */ }
pub struct Cx { /* non-owning execution capability; !Send, !Sync */ }

impl Cx {
    pub fn scope<T>(&mut self, f: impl for<'s> FnOnce(&mut Cx, Scope<'s>) -> T) -> T;
    pub fn bind<'g>(&'g self, root: &Root<'_>) -> Ref<'g>;
    pub fn alloc_real(&mut self, n: usize, out: Out<'_>) -> RResult<()>;
    pub fn apply(&mut self, f: &Root<'_>, args: Args<'_>, out: Out<'_>) -> RResult<()>;
}
```

`RootSlotId` includes scope/isolate identity in checked implementations. A root belongs to its scope, not to a Rust local's address. `Scope` owns stable slots separately from `Cx`, so keeping a root does not hold a mutable borrow of the execution capability. `scope.root(reference)` copies a live pointer into an empty precise slot without collection or heap mutation. Growing a root slab uses host allocation with no R callback and does not collect; exhaustion follows a separately audited path. Root slots cannot be overwritten or cleared while handles borrowed from that slot remain live. A handle cannot escape the generative scope. `Out` transfers values into caller-owned storage before a callee scope disappears.

A scope does not prove same-isolate use merely through an ordinary lifetime. Enforce an invariant isolate brand where practical and check owner identity when binding roots otherwise. This check is essential with nested isolates. The fast VM path uses slots whose owner is established at frame entry.

`Ref::view()` produces typed variants for every supported SEXPTYPE. Standard-vector views expose `&[T]` tied to the shared `Cx` borrow. Field reads can return another `Ref` within that same no-mutation interval without registering another root. Keeping a child across mutation first roots it; rooting its former parent is insufficient after the edge is replaced.

All allocation, GC, mutation, warning/error signalling, finalizer delivery, active binding access, promise forcing, package ALTREP access and C/R calls take `&mut Cx`. Standard type/header reads and own-class proven pure operations take `&Cx`. Generic ALTREP Length and Dataptr_or_null are effectful even if their names sound like reads. A root can be held in a `Vec`; a `Ref` in a `Vec` is harmless only within its borrow interval. No source-text container ban is needed for safety.

Borrowed slices never span a callback. Prepare or materialize through a rooted input, then borrow. Kernels that encounter a signalling point return an owned progress record, release slices, signal immediately, then reacquire data according to GNU R's continuing algorithm. Do not snapshot or restart the whole computation when handlers can alter inputs.

##### Mutation and sharing

REFCNT remains GNU R's semantic count, independent of GC roots. The current counted store is `crates/robigo-core/src/lib.rs:605`; GNU R's deferred stack protection is explicit at `R/src/main/eval.c:991` and `INCLNK_stack_commit:1012`.

Port `MAYBE_SHARED`, TRACKREFS, shallow/deep duplication, argument-list link accounting, `applydefine` and `R_CleanupEnvir` at the corresponding C operations. Never equate a single GC root with exclusive semantic ownership. A borrowed value retained across index forcing can need sharing protection even without a second heap edge. Commit deferred links before potentially mutating operations, and release them at the matching stack boundaries. AST temporaries obey the equivalent evalList rules.

Replacement builtins consume rooted argument slots through `Args::take_into`; this avoids an unnecessary heap-edge increment. They do not remove a visible binding merely to manufacture uniqueness. Validation and mutation order follows the C function, including partial effects where GNU R has them. Replace the current AGENTS.md Rc rule with these rules and source references.

`WriteLease` grants exclusive borrowed payload access only after duplication and storage preparation. `edit_pair(x, y, kernel)` checks aliasing before returning one mutable and one immutable slice. Object vectors expose setters, not unrestricted `&mut [Sexp]`.

An unpublished builder initializes pointer fields to valid sentinels, registers the object before any safepoint, and uses a construction store that skips old-edge decrements and unnecessary barriers while still establishing correct new-edge counts. Numeric uninitialized memory is `MaybeUninit<T>` until filled. No safe reference reads uninitialized elements. Publication consumes the builder. A fresh object that survives a collection is no longer automatically barrier-free: either forbid collection during the fill lease or update its generation/barrier state.

#### 2. Precise rooting and collector policy

Rust roots comprise scope slots, live VM slots, JIT root maps, active context fields, pending exits, isolate globals and code constants. The collector scans initialized live pointer fields only. A dead slot must become non-rooting before the next observable `gc()`. RAII scope lifetime alone is insufficient; a builtin that holds every intermediate until return can still delay a finalizer.

Generated code publishes every live SEXP before an allocating, signalling, callback or interrupt helper. Derived data pointers have a separately rooted owner. Rust LLVM registers and derived forms play no role in reachability. Native helpers cannot secretly collect under a no-safepoint signature.

Keep bounded conservative C segments from `Core::close_scan_segment` and the native trampolines. Existing regressions include both unwanted retention and missed stack-passed roots (`docs/ledger/core.md:7`). Never move conservative marking after weak classification. The current order is visible in `crates/robigo-core/src/gc.rs:1524`: precise tracing, conservative scan, weak processing, CHARSXP pruning. C scan retention remains a compatibility limitation to measure, not a proof of exact finalization for every buggy package. Correctly protected C packages are also tested with the scan disabled. Do not expand the scan to Rust.

Relevant implementation precedents are V8 `src/handles/handles.h`, Julia `src/llvm-late-gc-lowering.cpp`, JSC `Source/JavaScriptCore/heap/ConservativeRoots.cpp`, and GNU R `src/main/memory.c`. These external paths are orientation references inherited from round 1, not source claims reverified in this round. Our safety argument rests on published precise slots and the local code, not on analogy with another engine.

Published SEXPs and C-exposed payloads remain non-moving. Reopen the memory bake-off with the real object layout, roots and package callbacks. Compare current mark-region with a non-moving generational variant using side mark/age metadata and a remembered set, plus a tuned tracing-library implementation if it satisfies the same contract. Retain eager RC/trial deletion as an experimental control. There is no evidence yet that it overturns the measured 1.65x instruction disadvantage (`docs/ledger/memory.md:85`).

Raw pointer-bearing data exposure marks its owner always dirty for young collection. VECTOR_PTR's removal reduces exposure; STRING_PTR remains warned about, and DATAPTR_RW exists. Do not infer that every C store is now intercepted. A minor GC cannot finalize a weak key solely because an old container was not traced. Weak processing must use generation-aware reachability, and explicit full `gc()` completes the required fixed point. Preserve fork-friendly side marks and quantify dirty pages.

Moving private SSA values is fine: they have no observable SEXP identity yet. A moving heap of published objects would need a pin/escape protocol whose coverage is not established by the new headers. I do not choose one.

#### 3. Environments: slots are the authority

The target environment has no package-visible FRAME/HASHTAB layout. Proposed initial 64-bit layout, with 16-byte common header:

```text
ENVSXP
 +0..15 common header
 +16 enclosure: Sexp
 +24 shape: traced internal descriptor
 +32 overflow: traced frame block or nil
 +40 mutation_version: u64
 +48 structure_version: u64
 +56 nslots:u32, capacity:u16, mode:u8, env_flags:u8
 +64 BindingSlot[capacity]

BindingSlot, 16 bytes
 +0 payload:u64                 object, scalar bits, or promise descriptor
 +8 symbol_id:u32
 +12 state:u8                  unbound/value/int/real/logical/delayed/forced/active/cell
 +13 flags:u8                  missing, locked, other binding flags
 +14 auxiliary_index:u16        cold metadata; sentinel means extended index
```

Large shapes use an extended metadata index, never a silently truncated u16. The symbol table gives a canonical SYMSXP for each dense ID. Shape descriptors map symbol IDs to slots and record initial formal order. Per-binding creation order lives in cold metadata when execution makes it dynamic. Versions use a defined rollover protocol that flushes dependent caches before reuse.

This moves ENCLOS from the old +24. `crates/robigo-core/src/lib.rs:177` currently fixes it there; R 4.6 no longer does. Keeping +24 is also a valid measured layout control, but not an API requirement.

Function frames tail-allocate the common small case. Overflow storage preserves slot indices. Large/user environments add an open-addressed lookup index. Enumeration records GNU R's bucket size, collision-chain and rehash behavior independently of that index. `ls(sorted=FALSE)`, `env.profile`, serialization and removal/reinsertion order still expose those choices. Reproduce `R_HashResize` (`R/src/main/envir.c:427`) rather than approximating order by a final hash sort. Compare the compact index plus logical-order metadata against actual chained cells; retain the faster exact choice.

Active/locked/missing state belongs to each canonical slot. Active reads/writes enter the evaluator. Locking a binding does not force a representation change. Deleting then recreating a binding creates a new binding identity where GNU R does. A retained internal varloc uses an opaque stable location record or a cell-backed slot; it cannot point into movable overflow storage.

Public `R_GetBindingType` reads these states, including nested promise-chain semantics and cycle errors from `R/src/main/envir.c:780`. `R_getVar` at `:2634` and binding constructors at `:3801` use the same module as R-level lookup. They do not invoke a HOST table. Hidden base-native FRAME/HASHTAB adapters may be rebuilt to use slot iteration. If any remaining supported operation exposes a mutable binding cell, promote that binding to a real cell and make the slot point to it. There is never a writable snapshot plus a slot awaiting synchronization.

Global cache entries preserve `R_GlobalCache` lookup semantics (`envir.c:618`) and use per-symbol invalidation, a search epoch and versioned weak targets. Own locals are indexed. Free-variable caches guard every skipped frame against a newly introduced binding. `findFun` must still skip non-functions and force promises where required. `<<-` starts at the enclosure, respects locks and active bindings, and falls back to global exactly as `setVar` does.

All native setter paths notify versions. Raw C writes into vector values do not alter binding versions, so cached vector content assumptions are invalid across C effects. Locked bindings do not make the referenced payload immutable.

#### 4. Closures, promises, matching, contexts and unwind

Keep a 48-byte closure candidate: header, FORMALS +16, BODY +24, CLOENV +32, internal code owner +40. Retain actual mutable language/pairlist objects for R reflection and deprecated C accessors. Compile a separate immutable IR tied to mutation dependencies. Mutating the body spine can invalidate code without changing BODY's pointer. Guard closure/formal versions and conservatively invalidate exposed graphs across native effects where raw stores defeat finer tracking.

Prototype a 48-byte typed promise:

```text
 +0..15 header; private tag in extra: unforced/object/int/real/logical
 +16 PRVALUE or scalar bits
 +24 PRCODE
 +32 PRENV
 +40 compiled-thunk descriptor or nil
```

The collector traces +16 only in object state; it always traces the applicable code/env fields. Today's unconditional node trace (`gc.rs:1349`) must be replaced by generated per-kind descriptors. GNU R already has typed expansion machinery at `R/src/main/memory.c:4396` and `:4441`. Box on an observer that requires an object and install the box in that canonical promise. Setters, duplicate, serialize, inspector, PRVALUE and the binding API share this expansion logic.

Virtual promises store expression, evaluation environment, missingness, force state, optional scalar result and compiled-thunk identity in the activation's traced metadata. A binding points to that descriptor. Materialization allocates one PROMSXP and replaces the descriptor with a forwarding entry used by every alias. Escaping activations move descriptors into heap-owned storage before stack reuse. Dots forwarding cannot create independent promises for aliases of one delayed argument.

The new API makes the observation contract stricter. `R_GetBindingType` distinguishes value/delayed/forced; it unwraps nested promises and can error on cycles. `R_DotsElt` actually forces (`envir.c:1647`), while `R_GetDotType` must inspect without forcing. An eager scalar does not become an ordinary value binding just because no PROMSXP was allocated. Constant defaults preserve their actual forced state. Immediate promise optimization needs no deferred warning queue.

Force exactly as `eval.c:forcePromise:954`: preserve recursive-force errors, interrupted state, warning order, pending-promise chain, result sharing, and clearing PRENV. Context restoration marks interrupted promises as GNU R does. The warning itself may exit before normal forcing begins.

Matching uses match.c's exact, partial and positional passes, dots rules and original call diagnostics. Cache match plans against verified formals structure and actual argument names. Builtin and closure arguments live in precise stack windows. Materialize canonical DOTSXP chains when an observer needs them; DOTSXP is a cons spine, not an extra wrapper whose CAR contains another argument list.

One stable logical context chain serves all tiers. Proposed `rt/context.rs` uses slab-backed records with stable IDs and C projection pointers. Each record contains call, function, closure environment, sysparent, argument recipe, on.exit chain, srcref, saved protection/vmax/root tops, handler/restart state, pending-promise top, visibility/depth and C trampoline association. Base Defn.h maps RCNTXT to this representation. No pointers into a reallocating Vec.

`Result<T, Unwind>` carries a small exit token. An isolate-owned stack of traced `PendingExit {kind,target,value,condition,call}` records carries payloads. A single overwritable pending slot is insufficient: `on.exit` and cleanup code can catch a nested error while the original exit still exists. Unwind continuations own their retained record. This deletes the bridge's FLOWS token conversion table, not ownership of outstanding exits.

Preserve `core/src/unwind.rs:1`'s rule: Rust returns, thin C exports jump only after Rust semantic frames have returned, and every callable C method goes through a matching trampoline. No longjmp crosses live Rust guards. Scope drops run on Rust error returns; context checkpoints restore foreign protection and allocator state. Port `context.c:R_run_onexits:120` and its restoration order. Calling handlers execute before the raising context disappears.

All warnings signal at the original C operation, including `warning.expression` and warn=2 (`errors.c:vwarningcall_dflt:444`). Delayed printing when warn=0 remains GNU R behavior. Tests must distinguish signalling from printing.

#### 5. Strings

Use canonical CHARSXP pointers with bytes, encoding and NA state; remove RString/RStr. A checked `&str` is available only when bytes are valid UTF-8 and the operation's encoding rules allow it. A UTF-8 mark is not validation. Translation happens where the GNU R implementation calls translateChar/translateCharUTF8, using one shared implementation and locale state.

Keep weak interning by bytes plus encoding. Test the current flat table against a control-byte table and separate pools for short strings; measure full paste/match/unique workloads, not just intern hits. Immutable cached hashes can be computed during byte production when doing so preserves the existing hash definition and wins. Do not make intermediates uncached without auditing every pointer-equality assumption.

Pointer equality is a same-representation fast path, not general R string equality. Mixed Latin-1/UTF-8/native strings can compare equal after normalization. `match` and `unique` need a matching hash normalization path. Keep NA_STRING distinct from the interned string "NA". Cache translation only with the relevant encoding and locale epoch.

`paste` determines result encoding and sizes, builds into reusable owned scratch, then interns final strings once. Output roots survive each later allocation. Compare high-cardinality strings, repeated strings, long strings, invalid bytes and mixed encodings. A cached symbol ID in CHARSXP's spare word is a candidate; it must be isolate-correct and cannot repurpose visible scratch while a supported accessor still exposes it.

R 4.6 hides TRUELENGTH, but that does not justify silently changing inspect, serialization or packages that still use declared warned APIs. Remove the package-visible scratch obligation only where the selected header contract actually removed it. Internal metadata is ours to rearrange.

#### 6. Object layout, vectors and attributes

Keep the current 16-byte header as the reference candidate: type +0, flags +1, gp +2, REFCNT +4 u16, private extra +6, attrib +8. Source: `crates/robigo-core/src/lib.rs:155`. Standard vectors initially keep length +16, logical truelength +24, data +32. Real payload alignment and C type sizes are fixed by the relevant ABI.

Build a second layout candidate with 8-byte-aligned 40-byte cons cells: header plus car/cdr/tag, no universal aux. Closures and typed promises stay 48 bytes, and code metadata belongs to the closure or separate code owner rather than every cons cell. This requires new size classes and start-bit resolution. It saves bytes on argument/AST spines but may worsen alignment or cache behavior. A third candidate compresses selected private fields only after the first two have attribution; pointer compression is not a requirement. Generate offsets, C inline definitions and GC descriptors from one layout specification, with C and Rust assertions. Public TYPEOF must never expose a private internal descriptor kind.

Owned vectors use inline storage. Writable custom-allocator storage remains a distinct owner/release path. Foreign read-only/shared buffers use ALTREP with writable access detaching exactly once. Compact integer/real sequences are actual ALTREP objects across every tier; a register may carry a descriptor only while no identity has been observed. The `1:2e8` gate includes native transit, element reads and serialization behavior.

Every generic ALTREP operation uses the appropriate method and trampoline. `Length` does not read elements. Do not forbid an ALTREP data method from evaluating R merely to simplify borrowing. Match GNU R's GC inhibition where required while still handling errors and callbacks safely. DATAPTR_RW is specifically documented for ALTREP Dataptr implementations (`R/doc/NEWS:627`); it is not a general promise that arbitrary writable exposure disappeared.

Growth follows `subassign.c:EnlargeVector:140`, including bounds warnings, sharing, names/dim handling and the default 1.05 growth factor at `:177`. Logical truelength and physical reserved capacity can be separate, but keeping an allocation where GNU R would visibly duplicate can alter tracemem. Do not claim invisible doubling without an identity/trace proof. Exact geometric growth is already amortized; optimize allocation and copying under that protocol first.

Attributes remain one canonical pairlist in the first target. Their deprecated raw accessors still exist. More decisively, `R_mapAttrib` at `attrib.c:2019` computes CDR after the callback because the callback may change it. A compact copied attribute map cannot reproduce this by iterating a stale snapshot. Any later compact representation must retain an equivalent live cursor and canonical exposure protocol. At present, short pairlists plus verified lookup caching are the better target.

#### 7. VM, compiler and native tiers

##### Shared semantics with explicit effects

Proposed files: `rt/ir/{op,effects,verify,lower}.rs`, `rt/vm/{exec,stack}.rs`, `rt/jit/{baseline,opt,deopt}.rs`.

Represent operations with explicit effect summaries: allocation, read/write environment, force, dispatch, signal, native entry, reflection, visibility and numeric policy. Unknown calls conservatively carry all relevant effects. Maintain ordering edges for effects; normal value dependencies alone cannot preserve R's promise and warning order. Verify the lowered IR's effect ordering and root maps before execution.

The VM remains a compact register machine for cold code and wasm. Use indexed environment slots directly. Pure temporaries may stay in registers; there is no second authoritative variable cache. An optimizer may scalar-replace a provably private activation, with an exact recipe to materialize it before observation.

Candidate register format:

```text
Reg, 16 bytes: payload:u64 at +0; tag:u8 at +8; padding/reserved at +9..15
Tags: Empty, Object, Int, Real, Logical
```

Compare with separate 8-byte payload and 1-byte tag arrays. Avoid NaN-boxing. Use explicit layout, never a handwritten mirror of an unspecified Rust enum. Stack segments remain stable across callbacks. Per-safepoint live maps or explicit dead-slot clearing determine roots; measure both and their combination.

Carry forward the semantic techniques listed in `docs/ledger/perf-vm.md:2048`: function lookup through local frames, pure closure inlining, strict-prefix caller forcing, exact index forcing order, guard and rejection memos, and the bit-identical row-vector matrix kernel. Rebuild Rc ownership tricks, SmallVec allocation and run_with code placement. The present flush contract is documented in `runtime/src/vm.rs:1`; canonical slots remove that obligation.

##### Native compilation and deoptimization

Native ABI candidate:

```rust
unsafe extern "C" fn entry(
    isolate: *mut Isolate,
    frame: *mut ExecFrame,
    args: *const Reg,
    out: *mut Reg,
) -> ExitCode;
```

`ExecFrame` publishes code owner, environment, context ID, PC, live-root map and register storage. Generate header offsets from core's layout constants. Root live objects before every helper that can collect or signal. Reload payload addresses after effects. A kept SEXP does not imply that its vector length or data pointer stayed unchanged.

Cranelift first compiles the shared IR with per-operation resume states. A hotter optimizer uses SSA, escape analysis and guarded specialization. LLVM is a measured backend candidate, not a required second semantics implementation. Compilation cost, startup, native-code memory and hot-loop speed all count.

Deopt recipes reconstruct live values, bindings, virtual promises, contexts, visibility, PC and the pending exit. Resume before the next uncommitted operation. A warning, callback or visible store cannot be replayed. The present whole-call transactional path (`jit/closure.rs:1`) remains valid when all writes remain private and rollback is complete. Do not extend replay beyond that proof.

Vector fusion is admitted only when warning order, exact floating-point evaluation, ALTREP callback order, allocation-observable behavior and all guards are preserved. First optimize callback-free, no-warning regions. Materialize at mandatory R/C observation boundaries.

Caches contain weak targets or explicitly justified strong semantic owners. Clear dead weak cache references before address reuse; retained cache targets must not keep R6 environments alive. `compile.rs:630` already records that failure. S3 caches validate class content/version and method lookup dependencies, not class-vector address alone. Attribute, formal and body graphs can mutate without changing their top pointer. Foreign effects invalidate untrackable assumptions.

#### 8. Isolate and crate boundaries

`Isolate` owns Core, roots, PPStack, VM stack, contexts, pending exits, symbol/base/global tables, code registry, handlers/restarts, options, RNG, locale, connections, DLLs, warnings, console, source references and interrupts. Immutable builtin definitions and numerical tables stay process-wide. Mutable singleton fields are isolate-local unless separately proven frozen; core currently traces mutable immortal children (`docs/core.md:310`).

`Cx` is a capability over a pinned raw isolate pointer, not an exclusive Rust reference to the entire heap. At native entry, suspend the capability and end field borrows. A callback creates a fresh capability over the same UnsafeCell-backed storage. There must never be overlapping `&mut Isolate` references fabricated from TLS. A debug entry-depth/lease check validates the unsafe boundary. The existing capi rule that no table borrow crosses callbacks is at `crates/robigo-capi/src/state.rs:24`.

One TLS read at external C entry finds the isolate; ordinary Rust and generated code pass it explicitly. Do not add a process-global sole-isolate shortcut before a measured benefit and an ownership proof. Signals only set flags; R executes when the evaluator polls.

Target dependency direction:

- `robigo-core`: raw layout, allocation, tracing, roots, CHARSXP storage, low-level trampolines.
- `robigo-rt`: object-safe facade, all R semantics, interpreter, VM and compiler, native-call service interfaces.
- `robigo-capi`: public headers and export wrappers calling the same fallible rt operations; C exports do not own duplicate semantics.
- `robigo-cli`: assembly, executable and test front ends.
- syntax, numerics and text remain object-independent libraries.

Move loader/connection services needed by rt into rt or a lower platform module so capi depends on rt without a reverse crate dependency. Small typed function tables may remain for platform callbacks; no value conversion layer returns.

After consolidation, remeasure real libR.so with inlined allocation against executable exports. The existing median +4.65% result (`docs/core.md:1060`, council item 15) is a baseline to beat, not a permanent packaging prohibition.

#### 9. Migration with a versioned oracle

1. Record the supplied 4.6.2 source and oracle build manifest. Create separate expected-output sets keyed by version/configuration. Run every existing test input against 4.5.0 and 4.6.2, classify oracle differences, and retain both records. Do not overwrite historical expectations or remove markers.
2. Land `robigo-rt`, layout prototypes and standalone runners on main. No dependency from rt to the old runtime. The old executable keeps its existing oracle and ratchet. The rt column starts with object, root, binding and minimal evaluator tests, so progress does not wait for all of base.
3. Build a vertical slice that runs all of `calls.R`, base condition machinery, C callbacks and GC. Compile required GNU R base natives against the new headers. Use no leaf reverse bridge: even apparently pure conversions can alter attributes, ALTREP calls, encoding and identity. Share already object-independent kernels directly.
4. Complete base bootstrap in dependency slices. Report exact missing internals using ROBIGO_BOOT_VERBOSE and a source mapping manifest. Each builtin port cites its 4.6.2 function and records its effect/root obligations. A compile-success count is not a conformance metric.
5. Port builtins by family onto the new API. Codemods may rename patterns and constructors; sharing, roots, coercion warnings, ALTREP and replacement order require source-guided review. Old Rust is an optimization reference. Require targeted oracle cases for every semantic change. No requirement to keep the new code close to the old line count.
6. Bring C API and all native/package modes onto rt, then VM and native tiers onto the common IR. The prototype API must already have passed representative arithmetic, string, replacement and callback-heavy ports before mass conversion.
7. Flip the default only when every previously passing test identity either passes the new oracle or has a reviewed, reproduced oracle-change classification; all unchanged behaviors remain green. All 42 baseline source installs and package test blocks retain coverage against the selected 4.6 package fixtures. New API tests must pass. Require final speed gates below. Delete the old runtime, bridge and HOST/twin/watch/hold conversion machinery in that flip. Preserve service exports and C trampolines.

Main carries the new work throughout; no long-lived integration branch. Freeze additions tied to Rc layout, RString and bridge mechanisms. Continue oracle tests, parser/numerics/text improvements and critical old-runtime fixes with tests. Do not spend a separate campaign moving old-runtime TLS that the new runtime replaces. Every new passing old-runtime test becomes an explicit rt target.

Two executables are not a mixed heap. A mixed graph is possible in principle, and current core.md §6 already has a cross-heap tracing protocol. Reject it because it preserves the identity and mutation reconciliation problem that G3 exists to remove, not because cycles are mathematically unsolvable.

Cutover parity is not the full G3 completion claim. The brief reports 600/622 conformance scripts and remaining corpus failures. Complete the full suite against 4.6.2, constant-memory compact sequences, byte round-trips and bounded leak gates before calling G3 complete.

#### 10. Prototype and measurement plan

Use `crates/robigo-rt` as the growing prototype, with experiment configurations selected at build time or in isolated benchmark drivers. Never mix layouts in a single heap. Freeze baseline binaries and oracle artifacts. Report equal-tier comparisons separately from fastest-tier results.

| Decision | Concrete candidates | Workload and deciding measurements |
|---|---|---|
| Rust root storage | stable scope slabs; pre-sized activation roots; compiler liveness maps | calls, nested callbacks, finalizers; root stores, scan bytes, Ir |
| Env storage | tail slots; separate frame; canonical cells | six calls idioms, dynamic assign/rm, escape, active/locked bindings; allocations, Ir, RSS |
| Binding metadata | 16-byte entries; split payload/tag/symbol arrays | lookup and mutation mix, enumeration, cache misses |
| Promise storage | boxed; typed canonical; virtual descriptor with one-time materialization | defaults, dots, binding inspectors, C mutation, allocations and Ir |
| VM registers | 16-byte slots; split arrays | scalar loops, binarytrees, stack scan cost, JIT spill cost |
| Nodes | current 48-byte universal nodes; 40-byte cons plus per-kind bodies | pairlist construction, recursive calls, parsing, package allocation traces |
| Collector | current mark-region; non-moving young generation; tuned alternative control | all memory workloads, package suites, C exposed pointers, weak keys, pauses and fork pages |
| Code generation | existing-style VM; common-IR VM; baseline/native optimized tiers | cold startup, calls, hot package functions, total compile+execute time |
| String table | current open-addressing; control-byte layout and short-string pooling | paste/match/unique, cardinality and encoding distributions |

Use staged factorial comparisons: first one change against the same reference, then likely interactions such as frame layout × GC, register layout × root maps, and virtual promises × effects. Keep rejected results and reasons. A correctness failure eliminates a candidate before timing.

Correctness cases include scope escape and cross-isolate compile failures; borrowed slice across mutation; parent edge replacement; GC after every allocation and callback; nested C→R→C errors and resumable unwind; warning handlers that change inputs/options; active binding recursion; delayed/forced/missing and nested/cyclic promises; retained varloc deletion/recreation; native mutation of class/formal/body spines; weak caches and address reuse; byte strings; ALTREP method call logs; and callbacks that change the attribute list during R_mapAttrib.

Use Miri for the non-FFI borrow/root modules, optimized poison/torture tests for runtime and C, GCC and Clang shim builds, plus AST/VM/JIT differential runs. Force deoptimization at every eligible boundary and eager materialization at every virtual-object boundary. A no-in-place mode is a useful differential control on ordinary value results, but exclude or separately compare tracemem, inspector counts and deliberate C reference mutation: those can observe duplication.

Measure callgrind instructions with N versus 2N differencing, data/cache misses, allocated objects and bytes by kind, duplicate counts, root stores, finalizer order, materializations, peak and plateau RSS, collection p50/p99/max, code size, startup and total wall time. Interleave release runs on the same machine. Never compare `--runtime both` with `--runtime robigo` baselines; the ledger found a 2.5% discrepancy (`perf-vm.md:73`). Quick builds are not performance evidence.

Existing commands to use after implementation include `scripts/perf.py run --metric instr --filter 'calls|fib|binarytrees'`, targeted `scripts/corpus.py run fastr --filter ...`, `scripts/corpus.py run capi`, and `scripts/cran-gate.py run`. Add explicit binary/oracle selectors to the runners if needed rather than pretending a new rt flag already exists. Preserve the command, compiler, CPU, source hash, environment and raw outputs with each result.

Prototype exit before mass porting:

- Every implemented slice matches the selected oracle's stdout, stderr and status, including immediate warning handlers and new binding/dots APIs. No raw unsafe blocks in the four representative builtin families.
- Every general-call idiom is no slower in instructions than today's same-tier runtime. Their aggregate is at most GNU R's, with a target of at least 20% lower instructions than the current runtime. An unmet target sends the measured hot path back to design; it does not justify bypassing a semantic case.
- Fib and binarytrees preserve same-tier instruction performance within the repository's 0.5% gate. Heap use is bounded after repeated cycles. Any RSS or p99 pause increase over 10% requires a new layout/policy experiment before selection, not an unexplained waiver.
- Compile-fail tests enforce borrow/root rules, optimized torture passes, finalizer sequence matches, and minimal eval/pairlist/extptr/unwind C cases run without any bridge.

Final flip requires the complete parity mapping from §9, no benchmark more than 0.5% slower against both frozen old-runtime and recent-main baselines, no statistically supported package-suite slowdown, and no unbounded memory growth. Report both instruction and wall results. A wall-only win with worse instruction counts remains a disputed design result until the project's gate is explicitly changed; do not hide it in a geometric mean.

#### 11. Ranked risks and unresolved questions

| Rank | Risk | Test or measurement that resolves it |
|---|---|---|
| 1 | The safe capability API permits stale handles or reentrant aliasing | Compile an actual API prototype first; scope escape, root reset, cross-isolate, nested entry and Miri cases; inspect every unsafe constructor. A comment is not the proof. |
| 2 | Virtual promises expose the wrong delayed/forced state | Compare every R 4.6 binding/dots inspector at each force/materialize/escape transition, including chains and cycles; eager-materialization differential mode. |
| 3 | Copy-on-modify loses deferred links or changes trace behavior | eval.c protocol audit; nested index forcing and active-binding cases; ordinary-value no-in-place comparison plus separate tracemem/inspect oracle tests. |
| 4 | Compiler deoptimization repeats or omits effects | Force deopt before/after every committed operation; handlers mutate locals, C mutates payloads, on.exit catches nested errors. Compare output and state. |
| 5 | Raw C stores invalidate assumed class/body/formal data without notification | Native mutation probes with retained pointers and callbacks; conservative foreign-effect invalidation as the initial exact baseline. |
| 6 | Precise roots and caches still delay finalizers | Last-use root maps; weak-target cache stress; existing planted-stale-word tests and R6 tests at varied depths and tiers. |
| 7 | A young collector misses old-to-young writes or weak reachability | Raw pointer exposure suite, always-dirty owner accounting, minor/full GC differential reachability, fork RSS. Do not ship until the protocol is complete. |
| 8 | Layout packing saves bytes but loses actual speed | Coupled allocator/frame/register experiments with cache misses, instructions and wall time. Never choose from sizeof alone. |
| 9 | The version upgrade hides regressions under regenerated outputs | Immutable old/new oracle records, source manifests, automatic case-identity mapping, reviewed classifications for every difference. |
| 10 | Bootstrap and builtin coverage stalls behind missing semantics | Count executable conformance slices and unresolved bootstrap dependencies. Port the missing source functions; do not add conversion fallbacks that conceal the gap. |
| 11 | GC trigger and inspector observables conflict with freer memory policy | Separate physical allocation from R accounting; oracle-test documented inspect/gc fields and finalizer order. If a policy cannot preserve required observables, reject it regardless of speed. |
| 12 | New native tiers or libR.so improve hot loops but hurt startup/packages | Total compile+execute wall time, cold startup, export microbenchmarks and package runs; retain the measured winning tier/linkage selection. |

The central decision is to make object identity, binding state and effect order single authorities. Static borrowing protects each read interval; precise roots protect liveness; GNU R's link protocol protects sharing. None substitutes for another. This separation permits an aggressive compiler and new layouts without weakening the contract that selects them.


### Fable

### G3 round 2: one heap, no promise objects on the call path, a collector that needs `&mut Cx` (Claude Fable 5.2)

Base: main `4cff0ceb`. Target: GNU R 4.6.2 (assumed, per the round-2 brief).
Read-only review; nothing was built or run. Paths without a prefix are in
the repository. `R462/` is the 4.6.2 source tree named in the brief
(`.../scratchpad/r462/R-4.6.2/src`). Guesses are marked "(guess)". Numbers
I propose as bars are proposals, not results.

This file stands alone. Sections A to D are what the brief asks for first;
sections 0 to 11 are the full design.

---

#### A. What changed from my round 1, and why

1. **Handle: plain `Copy` `Value` plus `keep()` is replaced by a branded
   `Ref<'g>`.** My reason for no brand was the cost of a lifetime on every
   value in 115k lines. The lead removed cost as an argument, and under
   plan B the builtins are written new anyway. The brand is the strongest
   static guarantee on offer, so I take it.
2. **Rooting: "precise roots decide, conservative words pin" is dropped
   for Rust frames.** The coordinator named its weak point: a missing root
   lets a finalizer run on an object Rust still holds. I replace it with a
   rule that makes the question disappear: **the collector runs only at
   calls that take `&mut Cx`, and allocation takes `&Cx` and never
   collects.** No Rust frame is scanned, conservatively or otherwise, and
   safe code cannot hold a reference across a collection. The two-pass mark
   survives only for C frames (§2.4).
3. **Lazy arguments stop being promise objects.** Round 1 kept a PROMSXP
   for every argument the caller could not evaluate eagerly. R 4.6 makes
   the binding's *kind* (value, missing, delayed, forced, active) a public
   concept (`R462/include/Rinternals.h:647-654`) and removes `PRVALUE` and
   `PRCODE` from the installed headers. So a delayed argument is now a
   slot state. A general closure call allocates nothing once its frame is
   recycled (§4.2).
4. **Migration: "one API over two backends" is withdrawn in favour of
   Opus's plan B** (new crates on main with their own ratchet column).
   My plan constrained the API to what `Rc` can express; the brand, cons
   cell calls and CHARSXPs cannot be expressed over it honestly. I add
   three things to plan B: the new runtime targets 4.6.2 only and the old
   one is never retargeted, a ratchet ladder that removes the boot cliff,
   and no reverse bridge (§9).
5. **One execution engine.** Round 1 kept an AST evaluator beside the VM.
   The new runtime compiles everything and has no AST tier (§7.1).
6. **Unhashed environments are shaped slot frames, not only call frames.**
   R 4.6 hides `FRAME`, `HASHTAB` and `ENVFLAGS` from packages, so nothing
   outside robigo can see the difference, and R6 objects
   (`new.env(hash = FALSE)`) get inline-cacheable field access (§3).
7. **The vector header loses `truelength`.** R 4.6 hides `TRUELENGTH`,
   `SET_TRUELENGTH`, `SETLENGTH` and `SET_GROWABLE_BIT`. A scalar becomes
   32 bytes (48 today, 56 in GNU R) and a string of up to 7 bytes becomes
   32 bytes (48 today) (§6).
8. Kept from round 1: GNU R's REFCNT with deferred link counts; variables
   in environment slots with typed (unboxed) values; one-word `RResult`;
   hashed environments in GNU R's representation bit for bit; static
   immortal objects; the differential in-place oracle; experiment A.

#### B. Positions

##### The four disputes

| dispute | position |
|---|---|
| 1. Handle | Opus's `Ref<'g>` branded by `&Cx`, made stricter: *collection* also needs `&mut Cx`. Then the brand proves the whole rooting property for safe code, not only "no reference across R code". Astra's scope brand pays a root store per value; Muse's and my round-1 handle are checked only by gctorture. |
| 2. Rooting | None of the four round-1 answers. Rust: no scan at all; precise roots (value stack, contexts, traced isolate fields) and a collector that cannot run while a `Ref` exists. C: protect stack precise, C frames as pins decided after weak processing (experiment A decides pins against strong roots). |
| 3. Unboxed promise and binding values | Both in the binding: a slot has a kind and a value type, so a forced scalar argument is 16 bytes in the frame and no object. Heap PROMSXPs (rare after §4.2) use GNU R's `PROMISE_TAG` in the value word, which needs the tracer change the coordinator checked. Registers-only (Muse) is the control variant in the prototype. |
| 4. Migration | Plan B (Opus), amended as in §9. I withdraw my two-backend plan. |

##### The coordinator's four tentative picks

1. **Plan B: agree, with amendments.** (a) `robigo-rt` ports from 4.6.2's
   C and is gated on 4.6.2's recorded output; the old runtime stays on
   4.5.0 and is deleted at the flip, so no retargeting work goes into code
   that dies. (b) The heap crate is forked too (`robigo-heap`), because
   the layout changes in §6 and the deletion of HOST, holds and twins
   cannot coexist with the bridge. (c) Builtins live in their own crate
   with `#![forbid(unsafe_code)]`. (d) No reverse bridge for leaf
   builtins: it buys ratchet marks that were not earned and puts
   conversion code back on the path we are deleting.
2. **Compile-time branding: agree, and go one step further.** As picked,
   `Ref<'g>` still needs a conservative scan of the innermost Rust segment
   to survive allocation, which keeps the derived-pointer hazard, the
   `Vec<Ref>` hole and nondeterministic finalizers on allocation-triggered
   collections. Make allocation unable to collect and all three are gone
   (§2).
3. **No deferred warnings: agree.** Every warning takes `&mut Cx` and is
   signalled where GNU R's C signals it. The price is a root for whatever
   the builtin holds across the warning, which is the `PROTECT` the C
   source already has at that line.
4. **Build every variant, fastest exact one wins: agree, with two
   conditions.** The bars are written down before the runs (§10.4), and
   "exact" is checked by the two torture modes of §11 (materialize every
   promise; never modify in place), not by the benchmark's output alone.

#### C. The boldest changes I now propose

Each: the end-state benefit, the measurement that proves it, what it
deletes.

| # | change | benefit | measurement | deletes |
|---|---|---|---|---|
| C1 | **Allocation never collects; collection needs `&mut Cx`** (§2) | Rooting of safe Rust is a theorem of the borrow checker. Zero root traffic between safepoints. Native code keeps objects in machine registers across allocating helpers. Finalizers are deterministic at every collection, not only at `gc()`. | "Deferred garbage": bytes allocated between two consecutive safepoints that the next collection frees. Bar: never above max(32 MiB, half the live heap) on any corpus, benchmark or package suite. Instructions per call against Opus's segment variant. | The conservative scan of Rust frames, scan segments for Rust, `keep`, strict mode, `tests/no_heap_refs.rs`, JIT spills before allocating helpers, stack maps. |
| C2 | **Binding kinds; promise objects only where a promise is shared** (§4.2) | 0 allocations per general closure call after frame recycling (GNU R: environment, one cons per binding, one promise per argument, the promargs list). | Allocations and instructions per iteration of each `calls.R` idiom; bar ≤ 0.6x GNU R instructions on the four plain-call idioms. `ROBIGO_MATERIALIZE=1` (every delayed or forced slot is a real PROMSXP) must give identical output on every corpus. | `PromiseVal::Scalar*`, `caller_promargs` as a correctness-critical path, `BIND_MISSING_DEFAULT`, `materialize_default`, the recorded capi gap on forced defaults. |
| C3 | **One engine: no AST evaluator tier** (§7.1) | One implementation of matching, promises, assignment and dispatch instead of two; every call site has an inline cache; the interpreter-against-VM identity gate has nothing left to compare. | Instructions to load base (today 421M for `--vanilla -e 1`, ledger core 2026-11-10); a run-once microbenchmark (`eval` of 1e5 freshly built calls) against GNU R; package-suite wall time. Fallback if it loses: a minimal `eval` for run-once calls only. | `interp.rs`'s evaluator half, `ROBIGO_COMPILE=0`, half of `compiled_matches_interpreted` (`crates/robigo-cli/tests/conformance.rs:405`). |
| C4 | **Unhashed environments are shaped slot frames; `FRAME`/`HASHTAB` shims go** (§3) | `self$x` on an R6 object is one shape compare and one load, where GNU R walks a pairlist. Lookup by name in an unhashed frame is O(1). | An R6-style benchmark (field read, field write, method call on 1e3 instances) and `calls.R`'s `e$f(i)`; `ls(sorted = FALSE)`, `as.list.environment` and `serialize` of environments against the oracle. | `robigo_core::env` shims (`crates/robigo-core/src/lib.rs:791-848`), `env/table.rs`, one-way "exposure" of frames to C. |
| C5 | **24-byte vector header** (§6) | Scalars and short strings shrink by a third; more objects per cache line and per block. | The bake-off harness (`experiments/memory`) and the prototype with the header as a parameter: instructions, D1 and LL misses, peak RSS on binarytrees, a boxed-scalar loop and a 1e6 short-string vector. Keep only if nothing is slower. | `OFF_TRUELEN`, CHARSXP `TRUELENGTH` scratch, the `set_truelen(0)` store in every allocation (`crates/robigo-core/src/gc.rs:671, 724, 764`). |
| C6 | **Builtins crate with `#![forbid(unsafe_code)]`, `#[derive(Trace)]` for isolate state, compile-fail tests for the API** (§1.7) | About 60k lines are memory-safe by construction, and a forgotten root in isolate state is a compile error. | The crate compiles with the attribute. A `trybuild` suite of misuse cases fails to compile. | The class of bug that gctorture exists to find, for everything except the evaluator core, the heap and the C API. |
| C7 | **4.6.2 is the only target of the new runtime; expected output is stored per target** (§9.2) | No work on retargeting code that is deleted at the flip. | Re-record every corpus with 4.6.2 first and count the cases whose expected output changed. | The need to move `robigo-runtime`, the bridge and `rlib/` to 4.6. |
| C8 | **One C thread-local with the initial-exec model** (§8) | The `libR.so` shape loses its measured TLS blocker without nightly Rust. | Re-run the ledger's 2026-11-10 experiment (90 benchmarks, startup, C API microbenchmarks). | Every `thread_local!` in the runtime, the heap and the C API. |

Considered and not proposed: a moving young generation (C1 makes it
possible in principle, since no Rust reference exists during a collection,
but anything a package has seen must stay pinned and frame recycling
already removes the dominant young garbage; measure young-collection cost
after the flip before reopening it); forced unwinding in place of
`setjmp` trampolines (it would run C++ destructors GNU R skips);
attributes in anything but a pairlist (`ATTRIB` is still reachable under
`ENABLE_LEGACY_NONAPI_FUNS`, `R462/include/Rinternals.h:1258-1260`, and
attribute order is observable).

#### D. The best idea from each other design, and the one I think is wrong

**Opus.** Adopted: the brand tied to the isolate borrow, with allocation on
`&Cx` and R or C entry on `&mut Cx`. It is the only round-1 handle whose
rule the compiler checks at no run-time cost. Also adopted: weak inline
cache words (a strong cache on a method closure keeps an R6 object alive
and delays its finalizer; my round-1 table had exactly that bug in its S3
row), the new-crate migration, `robigo-text` as shared work.
Wrong: the conservative scan of the innermost Rust segment. It makes
soundness depend on LLVM leaving a recognizable pointer in a frame, it
needs a source scanner to forbid `Vec<Ref>`, and it leaves finalizers
nondeterministic on allocation-triggered collections. It exists only
because allocation may collect. Remove that and the scan has no purpose.

**Muse.** Adopted: "every safepoint takes `&mut Cx`, every view borrows
`&Cx`", which I extend from views to all references and from R code to the
collector. Also adopted: the unchecked fill path for fresh objects (a
store into an unpublished list needs no barrier and no decrement), and a
Rust root stack separate from `R_PPStack` (C may `UNPROTECT` across a
callback).
Wrong: allocation as a safepoint. With an unbranded `Copy` `Value` it
means every value held across any allocation must be rooted by hand, in
60k lines, with gctorture as the only check. That is GNU R's `PROTECT`
tax and rchk's bug class, imported.

**Astra.** Adopted: precise roots are not enough for exact finalizers; a
dead value in a registered slot delays one just as a stale stack word
does. So safepoints carry liveness maps (§2.3). Also adopted: compile-fail
tests for the API, context records at stable addresses, comparing equal
tiers in the prototype.
Wrong: `Value<'s>` whose every creation registers a root in a scope. It
is sound, but it pays a store for each field read in the code that today
pays nothing, to protect against a collection that C1 shows need not be
possible there.

---

#### 0. The design in one page

- **One heap** (`robigo-heap`, a fork of `robigo-core` with the bridge's
  machinery removed). Objects never move. Every object C sees is the
  object Rust uses.
- **Three kinds of code, three rooting regimes.**
  1. Builtins and library code (`robigo-base`, most of `robigo-rt`): safe
     Rust on `Ref<'g>`. The borrow checker proves no reference survives a
     point where R code, C code or the collector can run.
  2. The evaluator core (VM, calls, contexts, JIT helpers): `unsafe`,
     raw pointers, every object live across a safepoint is in a value
     stack slot, an environment slot or a context record.
  3. Package C code: `PROTECT`, plus its frames as pins.
- **A closure call** writes its arguments into the slots of one recycled
  frame object. A slot is 16 bytes: a symbol with state bits, and a value
  that is an object, an unboxed scalar, or (for an unevaluated argument)
  the environment to evaluate in. No promise, no cons cell, no argument
  vector.
- **Copy-on-modify** is GNU R's REFCNT with its deferred link counts for
  stack values (`R462/main/eval.c:997-1016`).
- **One engine**: source to VM code on first use; Cranelift for hot code;
  LLVM later. Native code needs no stack maps and no spills on the
  allocation path.
- **Migration**: new crates on main, own ratchet column against 4.6.2,
  flip when they contain the old ratchets and nothing is slower, delete
  the old runtime, the old core, the old C API crate and the bridge in the
  flip.

#### 1. The Rust-side object API (council item 24)

Crate `robigo-rt`, module `obj/`. Its interface is about ten types. It
hides every offset, flag bit and count.

##### 1.1 Types

```rust
/// The isolate: heap, evaluator state, stacks (§8). !Send, !Sync.
pub struct Cx { /* §8 */ }

/// Any live object. Valid until the next use of `&mut Cx`.
###[repr(transparent)]
###[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ref<'g>(NonNull<Header>, PhantomData<&'g Cx>);

/// A slot of the value stack: a precise root. Copy. Not tied to a borrow of Cx.
pub struct Root<'s>(NonNull<Slot>, PhantomData<&'s Scope>);
pub struct RootMut<'s>(/* a Root one may overwrite */);

/// A builtin's activation: its call, argument run, environment and root scope.
pub struct Act<'a> { /* slots base, count, names, call, env, *mut StackHdr */ }

pub struct Sym(&'static SymbolObj);          // immortal, unbranded
pub struct Global(u32);                      // index into the isolate's traced table
pub struct Unwind;                           // zero-sized error token (§4.6)
pub type RResult<T> = Result<T, Unwind>;

pub type BuiltinFn = for<'c, 'a> fn(cx: &'c mut Cx, act: &Act<'a>) -> RResult<Ref<'c>>;
```

- `Ref<'g>` is one word and non-null, so `RResult<Ref<'c>>` is one word
  and returns in a register.
- Immortal constants are `Ref<'static>` (`NULL`, `NA_STRING`, `TRUE`,
  `MISSING_ARG`, ...). They are static objects in the binary (§6.4), so
  `x == NULL` compares against a link-time address.
- `Root` reads through a raw pointer into the value stack, which is a
  separate mapping and not part of `Cx`'s borrowed memory:
  `root.get(&cx) -> Ref<'_>`.

##### 1.2 The three rules

1. **Allocation and every read take `&'g Cx`** and return `Ref<'g>`:
   `g.alloc_real(n)`, `g.cons(a, b)`, `g.mk_char(bytes, enc)`,
   `x.attrib()`, `l.get(i)`, `c.car()`. They run no R code, no C code and
   no collection.
2. **Everything that can run R code, C code or the collector takes
   `&mut Cx`**: `cx.eval`, `cx.apply`, `cx.force`, `cx.call_native`,
   `cx.warning`, active bindings, package ALTREP methods, `cx.safepoint()`.
   A `&mut Cx` borrow ends every `'g`, so a `Ref` cannot be used after it.
3. **What must survive rule 2 is in a root**: an argument (`act.arg(i)`),
   `act.root(r)`, a `RootMut` reused across loop iterations, a traced
   field of the isolate, or the heap.

A function on `&'c mut Cx` returns `RResult<Ref<'c>>`. The result keeps
`cx` borrowed, so the caller roots it, consumes it, or returns it before
using `cx` again. For "evaluate, then finish with pure work", the `_g`
variants return the shared borrow with the result:
`let (g, v) = cx.eval_g(e, env)?;`.

A builtin that calls R code in a loop:

```rust
// R: apply.c:do_lapply
fn do_lapply<'c>(cx: &'c mut Cx, act: &Act<'_>) -> RResult<Ref<'c>> {
    let x = act.arg(0);
    let n = x.get(cx).len();
    let ans = act.root(cx.alloc_list(n).finish());
    let mut elt = act.slot();                         // one reusable root
    for i in 0..n {
        elt.set(x.get(cx).as_list().get(i));          // a read; the borrow ends here
        let (g, v) = cx.apply_g(act.arg(1), &[elt.root()], act.env())?;   // R code ran: only roots survived
        ans.get(g).as_list().set(i, v);                // store: barrier and counts
    }
    Ok(ans.get(cx))
}
```

A pure builtin takes the shared borrow once and never roots anything:

```rust
fn do_rev<'c>(cx: &'c mut Cx, act: &Act<'_>) -> RResult<Ref<'c>> {
    let g = &*cx;
    match act.arg(0).get(g).view() {
        View::Real(x) => { let xs = x.slice(g)?; Ok(g.real_from_iter(xs.iter().rev().copied())) }
        ..
    }
}
```

##### 1.3 Matching and typed vectors

```rust
pub enum View<'g> {
    Null, Sym(Sym), Char(Char<'g>),
    Lgl(Lgl<'g>), Int(Int<'g>), Real(Real<'g>), Cplx(Cplx<'g>), Raw(Raw<'g>),
    Str(Str<'g>), List(List<'g>), Expr(List<'g>),
    Pairlist(Cons<'g>), Lang(Cons<'g>), Dots(Cons<'g>),
    Closure(Clo<'g>), Builtin(Prim), Special(Prim), Env(Env<'g>), Promise(Prom<'g>),
    ExtPtr(ExtPtr<'g>), WeakRef(WeakRef<'g>), Obj(S4<'g>), Bytecode(Ref<'g>),
}

impl<'g> Real<'g> {
    pub fn len(self) -> usize;                       // standard or robigo's own ALTREP classes
    pub fn as_slice(self) -> Option<&'g [f64]>;      // no method call
    pub fn slice(self, g: &'g Cx) -> RResult<&'g [f64]>;  // own classes materialize; package ALTREP: error to the caller's slow path
    pub fn elt(self, i: usize) -> f64;               // own classes
    pub fn regions(self, f: impl FnMut(usize, &[f64]));   // ITERATE_BY_REGION, own classes
}
impl Cx {
    /// Package ALTREP (methods are C): Dataptr under `&mut`, then a pure section.
    pub fn with_data<T, R>(&mut self, x: Root, f: impl FnOnce(&Cx, &[T]) -> R) -> RResult<R>;
    pub fn with_mut<T, R>(&mut self, x: Root, f: impl FnOnce(&Cx, &mut [T]) -> R) -> RResult<R>;
}
```

- `view()` is one byte load and a jump.
- `as_slice()` tests one flags byte (`ALTREP | EXTDATA | WRITING == 0`),
  then the length at `+16` and the data at `+24`.
- A package class's `Length`, `Elt` and `Dataptr` are C, so they are
  behind `&mut Cx` and are called exactly where GNU R calls them. The
  bridge's eager copies and the deferred-wrapper list of core.md §6 have
  no successor.

**Why a slice is sound for all of `'g`.**

| hazard | why it cannot happen during `'g` |
|---|---|
| the object is freed | the collector needs `&mut Cx` |
| R code or C code writes it (`REAL(x)[i] = ..`) | both need `&mut Cx` |
| ALTREP materialization moves the data | package methods need `&mut Cx`; robigo's own classes materialize into a second buffer and never move the first |
| the builtin writes it through another handle | writes to an existing object go through `cx.with_mut` (`&mut Cx`, so no slice is alive), which sets the header's `WRITING` bit; `as_slice` refuses an object with the bit, so `x[i] <- x` cannot alias |
| a fresh vector is written while read | `New<'g, T>` is not `Copy` and is the only handle until `finish()` |

So `&'g [f64]` satisfies Rust's aliasing rules, not only GNU R's
conventions. The one case round 1 "accepted as GNU R accepts it" (C
writing while Rust holds a slice) is now impossible to write.

##### 1.4 Mutation and copy-on-modify

`REFCNT` is GNU R's: heap edges and bindings count, locals and stacks do
not (`Sexp::store`, `crates/robigo-core/src/lib.rs:605`). The API uses GNU
R's predicate names.

```rust
impl Cx {
    pub fn alloc_real(&self, n: usize) -> New<'_, Real>;       // fresh, unique
    pub fn maybe_shared(&self, x: Ref) -> bool;                // REFCNT > 1
    pub fn shallow_duplicate<'g>(&'g self, x: Ref<'g>) -> Ref<'g>;   // reports to tracemem
}
impl<'g> New<'g, List> { pub fn set(&mut self, i: usize, v: Ref<'g>); }  // incref only: no barrier, no old value
impl<'g> List<'g>      { pub fn set(self, i: usize, v: Ref<'g>); }       // Sexp::store
```

- A replacement builtin decides as its C source does, for example
  `subassign.c:do_subassign_dflt` (`R462/main/subassign.c:1617`):
  `if MAYBE_SHARED(x) || (!IS_ASSIGNMENT_CALL(call) && MAYBE_REFERENCED(x)) { x = shallow_duplicate(x) }`.
- `AGENTS.md` rule 5 ("take arguments out with `mem::replace`") is
  retired. Arguments are slots; counts say whether a write is allowed.
- **A dead container keeps its children's counts**, as in GNU R. So the
  new runtime copies in a few places where today's `Rc` does not.
  `tracemem` and `inspect` show it, so matching GNU R here is the
  contract. `R_CleanupEnvir` and `unpromiseArgs`
  (`R462/main/eval.c:2128,2165`) are what lowers counts at call exit;
  frame recycling (§4.4) does their work exactly.
- **The link protocol** is GNU R's `R_BCProtCommitted` /
  `INCLNK_stack_commit` (`R462/main/eval.c:997-1016`), on the value stack:
  the isolate keeps `links_committed`; before an in-place modification of
  an object reached through a binding the evaluator commits (increments
  the count of every object slot between the mark and the running
  activation's base); a resuming activation uncommits. One compare when
  there is nothing to do. Within one activation the compiler emits
  `IncLnk`/`DecLnk` for a variable's value that is live across an
  assignment, as GNU R's compiler does. This is what makes
  `c(x, {x[1] <- 0; x})` copy.

##### 1.5 Errors and warnings in pure code

- **Errors are values until they reach `&mut Cx`.** `g.error(call, msg)`
  on `&Cx` records an unsignalled error in the isolate's pending record
  and returns `Unwind`. The first dispatcher or context boundary the `Err`
  reaches signals it (calling handlers run there) and then unwinds. No
  R-visible state changes between the raise and that boundary, because
  the context stack is the same, so handler order is GNU R's.
- **Warnings are immediate** (`cx.warning(call, msg)?`, `&mut Cx`). A
  kernel that warns once at the end (integer overflow, coercion NAs) sets
  a flag in its pure section, roots its result and warns, as
  `arithmetic.c` does. A kernel that warns in the middle and continues
  holds roots across the warning and re-borrows.

##### 1.6 Strings in the API

`Char<'g>`: `bytes() -> &'g [u8]`, `enc()`, `is_ascii()`, `is_na()`
(pointer compare), `as_str() -> Option<&'g str>` (validity cached in two
header bits), `to_utf8(g)` and `to_native(g)` at the points where GNU R's
C calls `translateCharUTF8` and `translateChar`. `Str<'g>::as_slice()`
gives `&'g [Char<'g>]`: the STRSXP payload read in place.

##### 1.7 What enforces the rules

| layer | code | enforcement |
|---|---|---|
| builtins | `robigo-base` (about 60k lines) | `#![forbid(unsafe_code)]`; the borrow checker |
| library code in `robigo-rt` (attributes, coercion, printing, deparse, dispatch, conditions) | safe modules | `#![deny(unsafe_code)]` per module |
| isolate state | every struct reachable from `Cx` | `#[derive(Trace)]` (`robigo-rt-macros`): a field that holds an object must be a traced type or carry `#[trace(skip = "reason")]`; a raw pointer field does not compile |
| the API itself | `obj/` | a `trybuild` suite: using a `Ref` after `cx.eval`, returning a slice past its borrow, holding `as_slice` across `with_mut`, storing a `Ref` in the isolate, mixing two isolates. Each must fail to compile |
| evaluator core, heap, C API | `unsafe` | gctorture at every safepoint with poisoned frees; the liveness torture of §2.3; the torture modes of §11 |

#### 2. Rooting and collection

##### 2.1 The decision

**The collector runs only inside a call that holds `&mut Cx`.** Allocation
holds `&Cx`, takes memory, and when GNU R's trigger says a collection is
due (`heapsize.rs`, unchanged) sets one bit in the isolate's `poll` word.

Safepoints, where `poll` is tested and a due collection runs:

- in the VM and in native code: at call entry, at a loop's backward jump,
  and after a builtin returns (one load and one branch each; the same
  word carries pending interrupts and pending finalizers);
- at the entry of every C API export that can allocate, when C calls it
  (§2.4);
- `gc()` and `cx.safepoint()`, which a builtin with `&mut Cx` may call
  inside a long loop.

Roots, all precise:

1. the value stack below its top, by the liveness maps of §2.3;
2. the context stack (§4.5) and the pending exit (§4.6);
3. the isolate's traced fields and its `Global` table;
4. `R_PPStack`, the precious list and the API's global variables, for C.

No Rust frame is read. A Rust frame cannot hold a live reference at a
collection: safe code by rule 2 of §1.2, the evaluator core by
construction.

##### 2.2 Why this and not the options in the brief

- **Conservative scan of Rust frames** (JavaScriptCore,
  `heap/MachineStackMarker.cpp`, `heap/ConservativeRoots.cpp`; file names
  from memory): the ledger already measured its cost here. Stale words
  delayed finalizers (core.md §3, lines 269-286; ledger 2026-10-28 and
  2026-11-11). Limiting the scan to the innermost segment (Opus) fixes
  `gc()` and leaves everything else.
- **Handle scopes on every value** (V8 `src/handles/handles.h`,
  SpiderMonkey `js/public/RootingAPI.h`): those collectors move objects
  and collect at allocation, so every local must be a handle. robigo's
  heap does not move, so a local needs protection only from being freed,
  and it cannot be freed if the collector cannot run.
- **What other systems do the same.** Erlang's BEAM lets a BIF that needs
  more heap than is free allocate a heap fragment and collects when the
  BIF returns (`erts/emulator/beam/erl_gc.c`, `erl_message.c`). Chez
  Scheme's allocator raises a collect request that the
  `collect-request-handler` serves at the next event check. Lua calls
  `luaC_checkGC` at fixed points of `lvm.c` and `lapi.c`, not from its
  allocator. The Rust crate `gc-arena` collects only between `mutate`
  calls and uses the same brand. (File names from memory.)
- **GNU R already separates the two for finalizers.** It runs them "only
  at points where running arbitrary code should be safe": the `evalcount`
  poll in `eval` and `bc_check_sigint`
  (`R462/main/eval.c:1091-1096, 6240-6247`), and `R_gc`
  (`R462/main/memory.c:3088-3103`). So when a finalizer runs relative to
  the allocation that made its object garbage is already "at the next
  safe point" in GNU R. Deferring the mark to the same point changes
  nothing a program can rely on.

Costs and how they are bounded:

- **Garbage made inside one Rust stretch is not reclaimed until the
  stretch ends.** Builtins mostly allocate their result, and loops that
  make garbage in volume call R code (a safepoint each time). C code is
  unaffected, since its allocation calls are safepoints. The metric is
  *deferred garbage* (table C1); `ROBIGO_GC_STATS` reports its maximum
  and CI gates it. A builtin that exceeds the bar gets a `cx.safepoint()`
  in its loop.
- **A hard memory limit reached inside a stretch** (`R_MAX_VSIZE`, or the
  OS refusing a large mapping) cannot be answered with a collection
  there. The allocation returns `Err` with the pending kind `NeedGc`; the
  dispatcher collects and re-runs the builtin if the builtin table marks
  it restartable (no effect before its first allocation), and otherwise
  raises GNU R's "cannot allocate vector of size" error. Tests run the
  conformance slices with a small `R_MAX_VSIZE`.

##### 2.3 Liveness: precise roots must also be exact

A dead value in a live slot delays a finalizer just as a stale stack word
does. GNU R's byte-code stack is exact because it pops. A register VM is
not, unless it says which registers are live.

- Every safepoint in VM code has a **liveness bitmap** in `CodeData`
  (registers live across that point). The collector scans a suspended
  activation by its saved pc's bitmap. No run-time cost.
- The alternative, clearing a register when its value dies, is the
  measured variant (§10, Q3).
- Builtin roots (`act.root`) are popped when the builtin returns: the
  dispatcher saves and restores the stack top, as GNU R checks
  `R_PPStackTop` around a builtin.
- **Liveness torture** (`ROBIGO_LIVENESS_TORTURE=1`): at every safepoint,
  every slot the bitmap calls dead is overwritten with a poison pointer.
  A wrong bitmap then fails at once.

##### 2.4 C frames

- `PROTECT`, `R_PreserveObject` and the API globals are precise roots, as
  today.
- An export that can allocate tests `poll` on entry. If a collection is
  due it first pushes its own `SEXP` arguments as precise roots (GNU R's
  API functions protect their arguments), collects, pops. Its body then
  runs as any Rust code does, with no further collection. So an export
  collects exactly where GNU R's function can allocate, and gctorture
  from R (`gctorture(TRUE)`) keeps its meaning for package C code.
- **C frames are pins, decided after weak processing.** The mark runs:
  precise roots and trace; weak references and finalizer readiness
  (`memory.c`'s `CheckFinalizers`) on those marks; then the conservative
  words of the open C segments, and trace; then prune the CHARSXP cache
  and sweep. The reorder is local to `collect_needing`
  (`crates/robigo-core/src/gc.rs:1524-1577`: `robigo_spill(scan_stack)`
  moves below `process_weak_refs`). GNU R is precise, so "precise roots
  decide finalizers" is GNU R's semantics exactly; the pin only keeps
  memory that buggy C still points at. A stale word in a C frame can no
  longer delay a finalizer, which closes the gap the ledger left open on
  2026-11-11 ("frames of package code below the cleared 1 KiB").
- If experiment A (§10.1) shows a package that relied on the stronger
  behaviour, C segments stay strong roots as today and nothing else in
  this design changes.

##### 2.5 VM register files, JIT frames, LLVM

- VM registers are value stack slots.
- Native code may keep objects in machine registers across any helper
  that only allocates, because allocation cannot collect. At a poll whose
  bit is set (the cold path) and before a helper that runs R code, it
  writes live objects to their slots; the deopt map already lists them.
  Objects do not move, so nothing is reloaded.
- No stack maps, no statepoints, for Cranelift and for LLVM. What LLVM
  does with a pointer inside a Rust or native frame no longer matters,
  because no frame is scanned.

#### 3. Environments

R 4.6 hides `FRAME`, `HASHTAB`, `ENCLOS`, `SET_FRAME`, `SET_HASHTAB` and
`SET_ENCLOS` (`R462/doc/NEWS:664-672`). Packages reach bindings through
`R_getVar`, `Rf_defineVar`, `R_envSymbols` and the binding API. The only
remaining constraint is observable order (`ls(sorted = FALSE)`,
`as.list.environment`, `eapply`, `mget`) and the bytes `serialize` writes
for an environment.

Two representations behind one ENVSXP, told apart by one header flag.

##### 3.1 Slot frames: every unhashed environment

Call frames, `new.env(hash = FALSE)` (R6 objects), frames built by C with
`R_NewEnv(.., FALSE, ..)`.

```text
ENVSXP, slot frame: 48 + 16n bytes
+0   header   flags: SLOTS, ESCAPED; gp: FRAME_LOCK
+8   attrib
+16  overflow   NULL, or a slots object for indices >= n
+24  ENCLOS
+32  shape      Shape: the symbols in slot order, and the index
+40  site       call frames: the caller's Code (resolves thunk indices, §4.2); else NULL
+48  slot[n]    16 bytes each

slot:  key   symbol address | state bits        val   an object, scalar bits, or an environment
key bits (indicative; the constants live in robigo-heap and the JIT imports them):
  0-1   kind      0 value · 1 active (val = function) · 2 delayed · 3 forced
  2-3   vtype     0 object · 1 REAL · 2 INT · 3 LGL      (kinds value and forced)
  4     LOCKED
  5     COUNTED   a delayed slot's environment edge is counted (§4.4)
  48-49 MISSING   GNU R's 0, 1, 2
  50-51 PRSEEN    0, 1 (under evaluation), 2 (interrupted)
  52    ORIGIN    thunk belongs to the caller's Code (site) or to the callee's (a default)
  53-63 thunk index (11 bits; a site with more uses a PROMSXP)
```

- Symbols are 64-byte aligned (§5), which frees the six low bits. User
  space addresses leave the top 16 free on x86-64 and aarch64; wasm32
  has 32 spare bits.
- The collector traces `val` when `vtype == 0`. That is the whole rule.
- **The fast path of a variable read** is
  `(key & 0x0f) == 0 → val`: a plain object value. Unboxed scalars are
  the next test. Delayed, forced and active go to one out-of-line
  function.
- **A `Shape`** is a heap object: the symbols in slot order, a parent, a
  weak transition table (symbol added, symbol removed), a small index for
  more than 8 symbols, and for a closure's root shape its `Code`. Defining
  a symbol the shape lacks moves the frame to the child shape and uses
  the next slot (the allocation's slack, then `overflow`). A frame that
  outgrows 128 slots or whose shape fans out past a limit gets a private
  shape that it mutates in place.
- **Order.** GNU R lists an unhashed frame newest first, and a call frame
  as "later definitions newest first, then the matched arguments in
  formals order". Both are functions of the shape's history, so the
  shape caches the listing order. For a call frame whose locals are
  preallocated but defined in a data-dependent order, a defining store
  records a rank (round 1, §3.2; a guess that under 5% of closures need
  it).
- **Reading a slot as an object** boxes an unboxed scalar and stores the
  box back: `R_expand_binding_value` (`R462/main/memory.c:4396`). `get`,
  `as.list`, a promise of the variable and C's `R_getVar` all go through
  it, which is what #276 needs.
- **`R_varloc_t`** (methods.so only, `tests/capi/symbols.tsv:593`) is
  `{env, slot address}` in robigo's `Defn.h`; its accessors are functions.

A three-formal call frame is 96 bytes in one allocation (today
`EnvData` is 240 bytes, `env/trace.rs:78`).

##### 3.2 Table frames: every hashed environment

The global environment, namespaces, `new.env()` (hashed by default),
`local()`. The object is a 48-byte node holding **GNU R's representation
bit for bit**: `+16` NULL, `+24` ENCLOS, `+32` HASHTAB (a VECSXP of
chains of binding cells), `+40` a version box or NULL.

- `envir.c` is ported as written: `R_Newhashpjw`, `R_HashSet`,
  `R_HashResize`, `R_HashSizeCheck` (`R462/main/envir.c:253,427,481`).
  Listing order and serialized bytes are GNU R's by construction.
  `env/table.rs` (360 lines that simulate that order) has no successor.
- Binding flags are GNU R's bits on the cell; typed cells use
  `BNDCELL_TAG` (`R462/include/Defn.h:464`).
- The base environment is the symbols' value cells, as GNU R.
- I considered shapes for hashed environments too (their listing order is
  also a function of insertion history). Rejected for now: the gain is
  about ten instructions on a lookup that is already O(1), and a per-site
  cache on the cell (§7.3) covers the monomorphic case.

##### 3.3 Lookup from compiled code

- Own locals: slot index, fixed at compile time, valid while
  `frame.shape == code.shape` (one compare per frame the VM did not make
  itself; none for the frame it made).
- Free variables in enclosing slot frames: an inline cache
  `{hops, shape per hop, slot}`. A frame whose shape lacks the symbol
  cannot bind it, so skipping is one compare per hop. This replaces
  today's guard memo misses on fresh frames (about 5,000 instructions per
  `calls.R` iteration, perf-vm ledger 2026-11-10).
- `obj$name` on a slot frame: `{shape, slot}`. One compare, one load, for
  every instance of an R6 class.
- Variables in table frames: the cache holds the cell, weakly (§7.3).
- From the global environment along the search path: GNU R's
  `R_GlobalCache` (`R462/main/envir.c:709-767, 1335`), stored per symbol
  (the symbol's word `+40`) and flushed per symbol when a `GLOBAL_FRAME`
  changes. It replaces `Interp::fun_cache` and the search epoch.

##### 3.4 Guards and versions

"Does `sym`, looked up as a function from here, still resolve to base?"

- Slot frames are skipped by shape.
- A table frame gets a **version box** on demand (a 16-byte object in
  `+40`, bumped when a binding that could hold a function changes). A
  memo holds the box and the version it saw.
- A locked namespace with locked bindings cannot change: static.
- Rule for every cache: **key on an object the cache keeps alive, or on a
  counter that never repeats; never on a bare address.**

##### 3.5 `<<-`

`sym <<- v`: the free-variable cache, then a slot write or a store into a
cell. `x[i] <<- v`: the in-place protocol on the enclosing binding after
`commit_links()`. In `calls.R`, `bal <<- bal + x` becomes two shape
compares and a scalar store with no allocation.

##### 3.6 The 4.6 binding API falls out

`R_GetBindingType` reads the slot's kind (or the cell's, as
`R462/main/envir.c:831-882` does). `R_MakeDelayedBinding`,
`R_MakeForcedBinding` and `R_MakeMissingBinding` write a slot kind, or a
PROMSXP into a cell. `R_DelayedBindingExpression`,
`R_DelayedBindingEnvironment` and `R_ForcedBindingExpression` read the
thunk's source and the slot's `val`. None of them creates a promise
object.

#### 4. Closures, promises, calls, contexts, unwinding

##### 4.1 Closures and code

```text
CLOSXP   48 bytes: +16 FORMALS · +24 BODY (always the source) · +32 CLOENV · +40 Code or NULL
LANGSXP  48 bytes: +16 CAR · +24 CDR · +32 TAG · +40 Code compiled for this expression, or NULL
Code     64 bytes, private type 27 (never reaches C or R; BCODESXP stays GNU R's serialized byte code)
         +16 consts   constants, thunk sources, shapes (traced)
         +24 source   +32 root shape   +40 next variant (debug, frame-free)
         +48 *CodeData (ops, thunk table, liveness maps, weak cache words, native entry)   +56 release fn
```

- The `function` special copies the `Code` from the `function` call's
  `+40` into the new closure, so closures made in a loop share code.
- `eval()` caches code in the expression's `+40`; it dies with the
  expression. The never-evicting `CACHE` thread-local goes.
- R 4.6 hides `SET_BODY`, `SET_FORMALS` and `SET_CLOENV`; the R-level
  replacement functions clear `+40`.

##### 4.2 Arguments are slot states, not promise objects

A compiled call site knows, per argument, a **thunk**: an entry in the
caller's `Code` with the argument's source expression (the same object
GNU R's `promiseArgs` puts in `PRCODE`, `R462/main/eval.c:3781`) and its
compiled code. Binding an argument is one of:

| argument | slot written in the callee's frame |
|---|---|
| a constant, or an expression the caller may evaluate early (today's `Code::strict` analysis) | `forced`, vtype by value, thunk index |
| anything else | `delayed`, `val` = the caller's frame, thunk index, ORIGIN = site |
| not supplied, formal has a default | `delayed`, `val` = the callee's frame, ORIGIN = callee, MISSING = 1 |
| not supplied, no default | `value`, `val` = `R_MissingArg`, MISSING = 1 |
| a promise object handed in (from `...`, from C) | `value`, `val` = the PROMSXP |

- **Forcing a delayed slot** sets PRSEEN to 1, runs the thunk in `val`,
  then writes `forced` with the value and clears PRSEEN.
  `eval.c:forcePromise` (`R462/main/eval.c:954`) is ported onto these
  bits: "promise already under evaluation", the PRSEEN 2 warning, the
  pending-promise stack.
- **`substitute`** takes the thunk's source through the frame's `site`
  (or the callee's own `Code` for a default). **`missing`** reads the
  MISSING bits and follows a symbol thunk into `val`, as `R_isMissing`
  does.
- **The edge from a delayed slot to the caller's frame is not counted**
  while the call is live: the caller is suspended below and cannot die.
  So a pending argument does not block the caller's frame from being
  recycled (GNU R reaches the same state through `unpromiseArgs`).
- **A PROMSXP is made only when a promise must be an object**: an
  argument passed into `...`, `delayedAssign`, lazy-load bindings in
  namespaces, `Rf_findVarInFrame` from legacy C, a site with more than
  2,047 thunks. When C or `...` asks for the promise of a slot, the slot
  becomes `value` holding a PROMSXP built from its state, once.
- **Heap PROMSXP layout**: `+16` PRVALUE (raw scalar bits when
  `PROMISE_TAG` is set, GNU R's `IMMEDIATE_PROMISE_VALUES`,
  `R462/include/Defn.h:38, 655`), `+24` PRCODE, `+32` PRENV, `+40` the
  thunk's `Code`. The tracer skips `+16` when the tag is set (it marks
  all four words unconditionally today, `gc.rs:1349-1354`).
- R 4.6's installed headers declare neither `PRVALUE` nor `PRCODE`
  (`R462/include/Rinternals.h:1258-1285` lists what remains: `PRENV` and
  `R_PromiseExpr`, both behind `ENABLE_LEGACY_NONAPI_FUNS`). So a
  package cannot observe where a promise keeps its value. This closes
  #277 and the recorded gap on forced defaults.

##### 4.3 `...`

A DOTSXP of cons cells holding PROMSXPs, as GNU R, in the first version:
rlang walks it with `CAR`/`CDR`, and the 4.6 dots API (`R_DotsElt`,
`R_GetDotType`, `R_DotDelayedExpression`) reads the same cells. One cons
and one promise per dots argument. The measured variant (§10, Q5b) puts
the promise state in the dots cell itself, one node per argument.

##### 4.4 Calls, and where frames go

- **Matching** is `match.c`'s three passes (`matchArgs_NR`,
  `R462/main/match.c:185`). A call site caches the result as a plan,
  valid while the callee's FORMALS is the same object.
- **A call**: resolve the function through the site's cache; take a frame
  from the free list for its size, or allocate; write the slots by the
  plan; push a context; run.
- **Frame exit** (on every path, including unwinding, in the context
  pop):
  - if the frame's count is zero and ESCAPED is clear: release the slot
    values (the decrements `R_CleanupEnvir` makes), and push the frame on
    the free list;
  - otherwise the frame survives: each delayed slot's environment edge
    becomes counted (COUNTED bit), so the caller's frame is kept by it.
- **ESCAPED** is set wherever a frame's address becomes a value without a
  counted edge: `environment()`, `sys.frame`, `parent.frame`,
  `sys.function`, `R_GetCurrentEnv`, the `rho` of `.External2`. In the
  evaluator core a frame is a `FrameRef`, a type that converts to an
  object reference only through `Cx::expose_env`, which sets the bit. So
  a missed hand-out is a type error, not a review finding.
- **Frame-free code.** Where today's analysis proves a closure's frame
  unobservable (pure-closure inlining, native call trees), the `Code` has
  a frame-free variant whose locals are registers. That carries over.

Allocations in a general call after warm-up: none.

##### 4.5 The context stack

One reserved region of `#[repr(C)]` records per isolate, at stable
addresses (C code compiled against robigo's `Defn.h` holds
`R_GlobalContext` pointers).

```rust
###[repr(C)]
pub struct Rcntxt {                 // R: Defn.h RCNTXT; robigo's Defn.h declares this layout
    callflag: i32, evaldepth: i32,
    call: Sexp, callfun: Sexp, cloenv: Sexp, sysparent: Sexp,
    promargs: Sexp,                 // NULL until asked for; built from the frame and the site's plan
    conexit: Sexp, handlerstack: Sexp, restartstack: Sexp, srcref: Sexp, return_value: Sexp,
    cstacktop: i32, flags: u32,
    vstack_top: *mut Slot, roots_top: *mut Slot, vmax: *mut u8,
    cjmpbuf: *mut JmpCtx,           // NULL for a context Rust opened
    cend: Option<extern "C" fn(*mut c_void)>, cenddata: *mut c_void,
}
```

- Its object fields are precise roots.
- `promargs` is built only for `sys.call`, `match.call`, `sys.function`
  or C. GNU R conses it on every call.
- `context.c` is ported onto it (`begincontext`, `R_run_onexits`,
  `R_restore_globals`, `R_sysfunction`: `R462/main/context.c:251, 120,
  170, 507`). It replaces `Interp::frames`, capi's `CONTEXTS` and
  `errors::Ctx`.

##### 4.6 `Unwind` and C

- Rust never jumps (core.md §5). Unchanged.
- The exit lives in the isolate, traced:
  `Pending { kind, target: *mut Rcntxt, mask, value, condition, call, signalled }`.
  `Unwind` is a zero-sized token. Today's `Flow` carries a `Value` and an
  `Env`, returns through memory, and its objects would be unrooted while
  `on.exit` code runs.
- This is the C API's existing model (`errors::Unwind`, the pending
  condition), so the runtime and the C API share one type and the
  `FLOWS` table goes.
- C ↔ Rust ↔ C is core.md §5 as it stands: an `extern "C"` wrapper turns
  `Err` into `longjmp` to the innermost C context; a trampoline turns the
  jump back into `Err`; each context passed runs its `conexit`.
  `Rf_eval` from C calls `Cx::eval` on the same objects. No conversion
  layer exists.
- The trampolines keep their cost (a `setjmp` per entry into C). I found
  no better end state: forced unwinding would run destructors GNU R
  skips.

#### 5. Strings

- `RStr`/`RString` have no successor. A string is a CHARSXP; a STRSXP
  element is 8 bytes (16 today); `NA_character_` is one static address.
- Layout (§6.1): `+8` hash `u32` and symbol id + 1 `u32`, `+16` length,
  `+24` bytes. A string of up to 7 bytes is 32 bytes (48 today).
- **`install(CHARSXP)` is two loads** through the cached symbol id. That
  is the path of `get("x")`, `e[["x"]]`, `x$name` with a computed name,
  `attr(x, "names")`, `as.name`, `do.call("f", ..)`.
- Symbols are 64-byte static or arena objects: `+16` PRINTNAME, `+24`
  SYMVALUE, `+32` INTERNAL, `+40` global-cache cell, `+48` id, `+52`
  hash. `Sym` is the address.
- **Hashing.** `unique`, `duplicated` and `match` hash and compare
  CHARSXP pointers when both sides are cached and their encodings allow
  it, as `unique.c` does. No string bytes are read.
- **`paste`** builds bytes in a scratch buffer and interns once per
  result. Interning measured at 0.59 of GNU R on a hit and 0.82 on a miss
  (ledger core 2026-10-28). Against today's uninterned `Rc<str>` a fresh
  long string may cost more; §10 measures it. Always interning stays:
  packages compare CHARSXP pointers.
- `TRUELENGTH` of a CHARSXP as package scratch is gone with the word
  (hidden in 4.6).

#### 6. Vectors and ALTREP

##### 6.1 Layout

```text
header  +0 type u8 · +1 flags u8 · +2 gp u16 · +4 refcnt u16 · +6 extra u16 · +8 attrib
vector  +16 length i64 · +24 payload                    (today: +24 truelength, +32 payload)
node    +16 car · +24 cdr · +32 tag · +40 aux            (48 bytes, unchanged)
```

- A scalar is 32 bytes. A length-2 or -3 double is 48. Granules stay 16
  bytes (core.md §2).
- R 4.6 hides `TRUELENGTH`, `XTRUELENGTH`, `SET_TRUELENGTH`, `SETLENGTH`,
  `SET_GROWABLE_BIT` and `STDVEC_DATAPTR`, and no base-package C file
  uses them (`rg TRUELENGTH R462/library/*/src` finds nothing). The
  replacements are `R_allocResizableVector`, `R_resizeVector`,
  `R_maxLength` (`R462/include/Rinternals.h:1211-1215`).
- Capacity comes from the allocation: the block's size class, or the
  large chunk's header. A vector made by `R_allocResizableVector` keeps
  the requested maximum in the last word of its slot, so `R_maxLength`
  returns what GNU R returns.
- `LEVELS`, `SETLEVELS`, `NAMED`, `OBJECT` are hidden too, so `gp` and
  the flags are private. They keep GNU R's values where `serialize`
  packs them.
- Payload alignment is 8 bytes. GNU R guarantees no more. (Guess: no
  package assumes 16; the package suites settle it.)
- **This is a measured decision** (C5). If the 24-byte header is not
  faster, the word stays and nothing else in the design changes.

##### 6.2 Storage, ALTREP, growth

- Storage variants: inline; EXTDATA (a foreign writable buffer); ALTREP
  (compact sequences, wrappers, deferred strings, foreign read-only
  buffers, package classes). HOST proxies and the bridge's `HOST_OWNED`
  go, with `hold`, `watch`, `held_roots` and `host_trace`. Two freed
  header bits become `WRITING` and `ESCAPED`.
- Compact sequences are robigo's own ALTREP classes everywhere. `for`
  reads them with no object per element. `RVal::Range` goes. This closes
  #460 and gives the `1:2e8` gate.
- Package ALTREP: each operation calls the class's method through the C
  trampoline, where GNU R calls it. `length(x)` calls `Length` only.
- **Growth.** `x[n + 1] <- v` is `EnlargeVector`
  (`R462/main/subassign.c:140`) with GNU R's growth rule. Beyond it,
  robigo extends in place whenever the allocation has room: a size class
  has slack, and a large vector's chunk reserves address space past its
  end. (Guess: reserving 2x above 1 MiB costs no resident memory; measure
  RSS.)
- **Attributes** are the pairlist at `+8`. `attrib.c` is ported once.
  The C API's `Rf_setAttrib` and R's `attr<-` call the same function.
- The S4 bit and OBJSXP are header state, which unblocks G6's first ask.

##### 6.3 What the C API crate becomes

`robigo-api` holds export wrappers, trampolines, headers, loading, the
graphics engine and `appl`. It ports no R semantics of its own: every
semantic entry point (`Rf_coerceVector`, `Rf_setAttrib`, `Rf_defineVar`,
`R_do_slot`, ...) is a wrapper over the `robigo-rt` function the builtin
also calls. It exports what 4.6.2's `libR.so` exports. The entry points
4.6 hides are not exported.

##### 6.4 Static immortals

`R_NilValue`, `NA_STRING`, `R_BlankString`, the logical singletons,
`R_UnboundValue`, `R_MissingArg` and the roughly 300 predefined symbols
are static objects in the binary. `x == NULL` and `tag == s::NAMES`
compare against link-time addresses. `mark_obj` already returns on
IMMORTAL before it looks at a block (`gc.rs:1256`); the store barrier's
slow path needs a branch for objects outside any block.

#### 7. VM and JIT

##### 7.1 One engine

```text
source (cons cells)  →  VM code (compiled on first eval or first call)
                     →  Cranelift native code (hot closures and loops)
                     →  LLVM (G8)
```

- `eval(expr, env)`: a symbol is a lookup, a constant is itself, a
  promise is forced, a call runs the `Code` in its `+40` or compiles it
  first.
- The compiler has two addressing modes: slot index (the closure's own
  frame) and by name through a cell cache (top level, `eval` in a table
  frame, code run in a frame it was not compiled for).
- Control flow and `<-` are compiled inline under guards on base's
  bindings, as today. The specials still exist as functions
  (`do.call("if", ..)`); as functions they call `cx.eval` on their
  arguments.
- **Debugging.** `debug(f)` and `browser()` stepping use a debug variant
  of the `Code` with a `Step` op before each statement. GNU R falls back
  to its AST evaluator for this; robigo needs no second evaluator.
- GNU R's two evaluators already had to agree for robigo's three tiers to
  be byte-identical today (`compiled_matches_interpreted`,
  `crates/robigo-cli/tests/conformance.rs:402-434`), so the VM alone can
  produce GNU R's output.
- Risk: code that runs once pays a compile. §10 (Q9) measures it. If it
  loses, a minimal direct `eval` for single calls is added, and nothing
  else changes.

##### 7.2 Registers

- The value stack: one reserved region per isolate (256 MiB of address
  space, committed on demand, a guard page; a fixed array on wasm32).
  16-byte slots: word 0 is a tag below 4096 (empty, REAL, INT, LGL) or an
  object; word 1 is the scalar's bits. Today's `RVal` is 24 to 32 bytes.
- No NaN-boxing: every 64-bit pattern is a double R can hold.
- The split layout (tags and payloads in separate arrays, Opus) is the
  measured variant.
- Variables are environment slots; there is no register cache of
  variables and no flush (`Vars`, `flush_pending`, `vm.rs:301-366` go).
- The dispatch loop keeps the pc, the frame's slot base, the environment
  and `cx` in machine registers.

##### 7.3 Inline caches

Data in `CodeData`. **A cache holds strongly only what cannot keep user
objects alive: `Code`, `Shape`, symbols. Everything else is a weak word**
the collector clears when its target is unmarked.

| site | cache | valid while |
|---|---|---|
| call | callee `Code`, matching plan | `callee.aux == Code` |
| own-frame variable | slot index | static |
| free variable | hops, shape per hop, slot | shapes match |
| `obj$name` | shape, slot | shape matches |
| variable in a table frame | cell (weak) | cell bound |
| global variable or function | the symbol's global-cache cell | not flushed |
| guard | version box (weak) and version | version unchanged |
| S3 dispatch | class attribute object (weak), method closure (weak) | same class object, method's cell unchanged |

S3 dispatch costs 18.8k instructions today (perf-vm ledger 2026-11-10).
A class vector that is the same object has the same contents while the
cache holds it, so the key is exact.

##### 7.4 What carries over from the perf-vm ledger

| technique | here |
|---|---|
| call-site function lookup through local frames | shape tests, then cell or global cache |
| pure-closure inlining, nested inlining, whole-closure native code, loop summaries | unchanged in design; frame-free `Code` variants |
| caller-forced promises | `forced` slots; now an optimization, not the way to avoid an allocation |
| index promises forced in R's order | unchanged |
| guard memoization | shapes and version boxes |
| static per-site rejection memos, `%*%` row-vector dot products | unchanged |

Redone: register buffers, `set_var`/`own_var`, `Vars`, the layout of
`run_with`.

##### 7.5 JIT

- ABI: `extern "C" fn(cx: *mut Cx, slots: *mut Slot, env: *mut u8) -> u32`.
- Object access by header offset: type `+0`, flags `+1`, count `+4`,
  attributes `+8` against NULL's static address, length `+16`, data
  `x + 24`. The constants are imported from `robigo-heap`, so a layout
  change is a compile error.
- Scalars live in machine registers in loops and are written to slots at
  exits. A scalar store to an environment slot is two stores (the key is
  a constant).
- In-place vector writes check `refcnt <= 1` at entry, after
  `commit_links()`.
- Deoptimization is unchanged: a loop leaves at op granularity with state
  written back; a call tree is abandoned and replayed by the VM.
- Rooting: §2.5. Nothing on the hot path.

#### 8. The isolate record

```rust
###[repr(C)]
###[derive(Trace)]
pub struct Cx {
    // hot, fixed offsets for the VM and native code
    poll: Cell<u32>,                        // gc due · interrupt · finalizers pending
    vstack: *mut StackHdr, links_committed: *mut Slot,
    ctx_top: *mut Rcntxt, eval_depth: i32, visible: bool,
    frame_free: [FrameList; 8],             // recycled frames by size
    heap: UnsafeCell<robigo_heap::Heap>,    // allocation through &Cx
    pending: Pending,
    global: Traced<Env>, base: Traced<Env>, base_ns: Traced<Env>, ns_registry: Traced<Env>,
    handler_stack: Traced<Obj>, restart_stack: Traced<Obj>, srcref: Traced<Obj>,
    globals: GlobalTable,
    // everything that is a thread_local! today
    shapes: ShapeRoot, numerics, rng, options, conns, s4, dlls, warnings, locale, tz, jit, ...
    api: ApiTables,                         // R_PPStack, precious set, DLL tables
}
```

- All 69 `thread_local!`s of the runtime and the C API's remaining ones
  become fields, or disappear with what they served (the symbol interner,
  the compact-sequence registries, the bytecode `CACHE`, the bridge's 42).
- **How code reaches it.** Rust gets `&mut Cx` or `&Cx`. Native code gets
  it as an argument. A C export reads **one thread-local, defined in the
  C shim with the initial-exec model**
  (`__thread struct Cx *robigo_cx __attribute__((tls_model("initial-exec")))`),
  once, and passes it down. That is one `%fs`-relative load in an
  executable and in a `libR.so` loaded at start-up. The ledger found that
  initial-exec removes the TLS cost but stable Rust cannot select it
  (ledger core 2026-11-10, lines 214-219); a C definition can.
  `scripts/lint.sh` fails on any `thread_local!` in these crates.
- **`libR.so` revisit** (core.md §12 item 15). Of the two measured
  blockers, TLS is gone. The out-of-line allocator shim shrinks: objects,
  frames, promises and argument runs no longer come from the Rust
  allocator. Re-run that ledger entry's experiment after the flip under
  its rule (no benchmark slower).
- `R_GlobalEnv` and the other per-isolate API variables stay exported
  variables mirroring the record until isolates run as threads (core.md
  §10). Then symbols' value and cache words move to per-isolate arrays
  indexed by symbol id; the layout leaves the id in the symbol.

#### 9. Migration

##### 9.1 The plans compared

| plan | verdict |
|---|---|
| narrow `Value` on main, then one swap on a long-lived branch (coordinator, round 1) | no: the swap is not local to `value.rs`, and the branch has no green ratchet for weeks |
| one API over two backends (mine, round 1) | withdrawn: the target API cannot be honest over `Rc` |
| a mixed Rc/GC heap | no, unanimously: persistent cross-heap references are the bridge, and the bridge is the source of #278, #277, #276, #189, #304, #167/#389 and #217 |
| **new crates on main with their own ratchet column; flip; delete (plan B)** | **yes** |

##### 9.2 Crates and targets

```text
crates/robigo-heap     the heap: fork of robigo-core's heap.rs, gc.rs, chars.rs, unwind, heapsize.rs;
                       new layout (§6.1), no HOST/hold/twin code, slot tracing, no collection in alloc
crates/robigo-rt       Cx, obj/ (the API), env, calls, contexts, conditions, compiler, VM,
                       attributes, coercion, dispatch, printing, deparse, serialize
crates/robigo-rt-macros  #[derive(Trace)]
crates/robigo-base     builtins, one module per GNU R C file; #![forbid(unsafe_code)]
crates/robigo-api      the C API over robigo-rt (§6.3)
crates/robigo-jit      Cranelift tier (not built for wasm32)
rlib46/                GNU R 4.6.2's base R sources
```

- The binary is `robigo-rt` until the flip. The old `robigo` is untouched
  and stays what users, the old ratchets and the perf gate see.
- **Targets.** Expected output is stored per target:
  `scripts/regen-expected.sh --target 4.6.2` records a second set beside
  the 4.5.0 one for every conformance script and corpus. The old runtime
  is gated on 4.5.0, the new one on 4.6.2. The first measurement of the
  whole project is the diff between the two sets (how many cases changed
  expected output).
- **Packages.** Robigo's installed headers become 4.6.2's. The census
  shows what that removes: `SETLENGTH` (5 packages), `SET_PRVALUE` (5),
  `PRVALUE` (4), `SET_GROWABLE_BIT` (4), `SET_TRUELENGTH` (4), `LEVELS`
  (1) (`tests/capi/symbols.tsv:210-252, 479`). The package corpus is
  re-pinned to releases that build on GNU R 4.6.2, and
  `scripts/capi-census.py scan` is re-run against them before any layout
  in §6.1 is frozen.

##### 9.3 The ratchet ladder (no boot cliff)

| level | what runs | ratchet |
|---|---|---|
| L0 | `--no-base`: the evaluator and primitives. The target list is found mechanically: the cases the *old* runtime passes under `--no-base` | `tests/r/passing-rt/`, `tests/corpora/*/passing-rt/` (marker files, same rules) |
| L1 | base loaded from `rlib46/` | same directories, more cases |
| L2 | default packages (stats, utils, methods natives through `robigo-api`) | same |
| L3 | the capi corpus and the CRAN package suites | `tests/capi/passing-rt/`, package ledger column |

- L0 exists from the first week of the evaluator, so builtin modules have
  a signal before base boots.
- How many builtins boot needs is a measurement, not a fear: count the
  distinct builtins the old runtime calls while loading base (a ten-line
  counter behind `ROBIGO_BOOT_VERBOSE`). Base's top level is mostly
  `name <- function` (guess: under 100 distinct builtins).
- CI builds both binaries and runs the `rt` column under gctorture and
  the torture modes of §11.

##### 9.4 How the code gets written

- **Port GNU R 4.6.2's C, with the old Rust beside it as a second
  reference.** The API is SEXP-shaped (`car`, `cdr`, `view`, `alloc`,
  `maybe_shared`), so a C function ports nearly line by line. A `PROTECT`
  that spans only allocation disappears. A `PROTECT` that spans an `eval`
  or a warning becomes a root, and the compiler finds every one that is
  missing.
- Representation-independent kernels are moved, not re-ported: exact
  arithmetic, x87 reductions, radix `order`, number formatting, regex.
  They go to leaf crates (`robigo-numerics`, `robigo-text`) that both
  runtimes call, so that work pays off before the flip.
- One owner per module (`docs/workstreams.md`). The module's corpus slice
  in the `rt` column is its number.
- **A second oracle against thin corpus coverage**: a differential
  harness runs the same generated calls of one builtin on the old and
  the new binary and reports differences. It finds regressions the
  corpora do not cover. GNU R remains the arbiter where the two disagree.
- No reverse bridge. Nothing in `robigo-rt` or `robigo-base` may depend
  on `robigo-runtime`; a lint enforces it.

##### 9.5 Order of work

1. **On main today, no new runtime needed**: experiment A (§10.1);
   re-record the corpora against 4.6.2; the census against 4.6.2
   packages; the boot-builtin count; the header-size run in
   `experiments/memory`.
2. **Prototype** (§10.2): the first commits of `robigo-heap` and
   `robigo-rt`, with the variants behind cargo features. Results in
   `docs/ledger/g3.md`. Losing variants are deleted when it exits.
3. **API freeze**: `docs/object-api.md`, the `trybuild` suite, three
   pilot modules in `robigo-base`.
4. In parallel: the evaluator up the ladder (calls and contexts,
   conditions and unwinding, base load, dispatch, `robigo-api`, JIT);
   builtins by module; kernels into leaf crates.
5. **Flip gates**, all of:
   - every marker in every old `passing/` directory is in the matching
     `passing-rt/`;
   - `scripts/perf.py compare`: no benchmark more than 0.5% slower than
     the old runtime; every `calls.R` idiom at or below GNU R;
   - the capi corpus and the package suites at or above the old numbers,
     and package wall time not worse;
   - design.md §5.6's G3 gate: `1:2e8` in constant memory, byte strings
     round-trip, leak tests bounded;
   - #278, #277, #276, #189, #304, #167, #389, #217, #460 and #140 closed
     with regression tests, nested callbacks included;
   - the `rt` column clean under gctorture, liveness torture,
     `ROBIGO_MATERIALIZE=1` and `ROBIGO_NO_INPLACE=1`; deferred garbage
     under its bar.
6. **Flip**: one PR makes `robigo-rt` the binary and deletes
   `crates/robigo-runtime`, `crates/robigo-core`, `crates/robigo-capi`
   (bridge included), `rlib/`, the 4.5.0 expected set and the old
   ratchets; renames the new crates; rewrites `AGENTS.md` (appendix B).
7. After: re-measure `libR.so` (§8); sticky-mark young collections behind
   the existing store barrier if pause or young-collection cost calls for
   it (R 4.6 leaves `STRING_PTR` and `DATAPTR` on pointer vectors only
   behind the legacy switch, so "always dirty" objects become rare).

The bridge dies in step 6. The old runtime needs it until then.

##### 9.6 What pauses on main now, and what continues

Pause:

- every feature and all performance work in `robigo-runtime`,
  `robigo-capi` and the bridge. A fix lands there only with a committed
  test, which is then a target for the new column;
- retargeting anything old to 4.6;
- new `Value`, `RString` or `thread_local!` work.

Continue:

- `robigo-syntax`, `robigo-numerics`, the `robigo-text` extraction, the
  REPL and CLI;
- conformance scripts and corpora (every new case is a target for both
  columns);
- base natives built from GNU R's sources (they bind to whichever C API
  crate is linked);
- S4/G6 design.

#### 10. The prototype and the measurement plan

##### 10.1 Experiment A: runs on main today

Move the conservative scan below weak processing in `collect_needing`
(§2.4), on today's core. Run the core and capi tests under gctorture, the
capi corpus, the ledger's finalizer stress script (2026-11-11:
`.Call('pointer')` at depths 0..20 by 0..20) with the naked stub and the
1 KiB zeroing removed, and the package suite.

- Decides: C frames as pins or as strong roots; whether the scan-segment
  stub and zeroing (+108 instructions per `.Call`) can go.
- Pass: no capi or package regression; the stress script misses nothing
  in any tier.

##### 10.2 The prototype

`robigo-heap` and `robigo-rt` seeds: the parser from `robigo-syntax`
building cons cells; the API of §1 with its `trybuild` suite; slot and
table frames; binding kinds; contexts and unwinding; the compiler and VM
(no JIT); scalar arithmetic and comparison; `[`, `[[`, `[<-` on atomic
vectors and lists; `c`, `list`, `$` on lists and environments; `<<-`;
`UseMethod`; conditions sufficient to run base's own `tryCatch` source;
`gc()`, `reg.finalizer`, weak references; a minimal `.Call` path.

Benchmarks: `fib`, `binarytrees`, each of `calls.R`'s idioms separately
and the file unmodified, an R6-style object benchmark, `paste`/`match`/
`unique`/`sprintf` loops, a run-once `eval` loop. Compared at equal tier:
today's robigo with the JIT off, and GNU R 4.6.2.

Metrics, in the perf-vm ledger's method: callgrind instructions per
iteration by N-against-2N differencing; estimated cycles
`Ir + 10·D1m + 100·LLm`; heap allocations and bytes per call by object
kind; peak RSS; longest pause; deferred garbage.

##### 10.3 Variants (all built; the fastest exact one wins)

| # | question | variants | decides |
|---|---|---|---|
| Q1 | call frame | inline slots in the environment; environment plus a separate slot vector (Muse); GNU R pairlist frame | §3.1 |
| Q2 | frame reuse | LIFO recycling with ESCAPED; young collections at the bake-off's knee; GNU R's trigger only | §4.4 |
| Q3 | liveness | bitmaps per safepoint; clearing registers at death | §2.3 |
| Q4 | collection points | never in allocation (C1); allocation collects with the innermost Rust segment scanned (Opus); allocation collects with scope roots (Astra) | §2.1 |
| Q5 | arguments | slot kinds (C2); a PROMSXP per lazy argument with `PROMISE_TAG` values (Opus); registers-only unboxing (Muse) | §4.2 |
| Q5b | dots | DOTSXP of PROMSXPs; promise state in the dots cell | §4.3 |
| Q6 | link protocol | deferred commit; eager counts on register writes | §1.4 |
| Q7 | strings | CHARSXPs against today's `RString`, on the four string loops | §5 |
| Q8 | register slots | 16-byte slots; split tags and payloads | §7.2 |
| Q9 | engine | compile everything; compile plus a direct `eval` for single calls | §7.1 |
| Q10 | vector header | 24 bytes; 32 bytes with `truelength` | §6.1 |
| Q11 | unhashed environments | shaped slots; GNU R pairlist | §3.1 |
| Q12 | error return | one-word `RResult` with `Pending`; `Flow` by value | §4.6 |

Each variant must pass the prototype's tests under the torture modes
before its number counts.

##### 10.4 Exit criteria, written before the runs

- **E1.** `calls.R`, no JIT: the four plain-call idioms and `<<-` at or
  below 0.6x GNU R's instructions with 0 allocations per call; S3
  dispatch at or below 0.8x; `tryCatch` at or below 1.0x; the unmodified
  file at or below 0.8x GNU R's wall time (today 1.23x).
- **E2.** `fib` and `binarytrees` at or below today's robigo with the JIT
  off, in instructions and estimated cycles; `binarytrees` peak RSS at or
  below today's and at most 1.1x GNU R's.
- **E3.** The finalizer stress script misses nothing at any depth in
  1,000 runs under ASLR; gctorture and liveness torture clean.
- **E4.** `ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1` give identical
  output on every prototype test.
- **E5.** Three real modules (`arithmetic.c`, `paste.c`, the
  `assign`/`get`/`exists` part of `envir.c`) ported into a
  `#![forbid(unsafe_code)]` crate, each within 1.2x the line count of
  its C source, reviewed by a council member who did not write it. The
  `trybuild` suite passes.
- **E6.** `paste`/`match`/`unique` at or below today's instructions, or
  the difference explained and accepted in the ledger.
- **E7.** Deferred garbage under max(32 MiB, half the live heap) on every
  prototype workload, including `paste`, `strsplit`, `unlist` and
  `sprintf` on 1e7 elements.
- **E8.** The capi corpus cases `eval`, `pairlist`, `unwind` and `extptr`
  pass with no bridge.
- **E9.** Loading base at or below the old runtime's instructions; the
  run-once `eval` loop at or below GNU R.

If E1 fails, the frame, argument or call design is wrong, and that is
known before any builtin is ported. If E7 fails, Q4's second variant is
the fallback and the rest of the design stands.

#### 11. Risks and open questions, ranked

| # | risk | what resolves it |
|---|---|---|
| 1 | The link protocol misses a case: an in-place write is visible through an uncounted reference. Silent wrong answers. | `ROBIGO_NO_INPLACE=1` makes every `MAYBE_SHARED` true. In-place modification is unobservable, so any output difference between the two modes over the corpora is a bug. `tracemem` cases against GNU R check the other direction. |
| 2 | A frame is recycled while something holds its address. Wrong bindings, silently. | `FrameRef` converts to an object only through `Cx::expose_env` (type-enforced). `ROBIGO_FRAME_TORTURE=1` poisons and quarantines every recycled frame. |
| 3 | A slot kind misses an observer (`substitute`, `missing`, `sys.function`, `match.call`, the binding API, legacy `findVarInFrame`). | `ROBIGO_MATERIALIZE=1`: every delayed or forced slot is a real PROMSXP. Output must be identical over the corpora and package suites. |
| 4 | Rooting errors in the unsafe evaluator core (a wrong liveness bitmap, an object in a Rust local across a safepoint). | gctorture at every safepoint with poisoned frees; liveness torture; the core is small and has one owner. |
| 5 | Deferred collection: a Rust stretch makes unbounded garbage, or meets a hard memory limit. | The deferred-garbage gate; `cx.safepoint()` in the offending loop; restartable builtins on `NeedGc`; tests under a small `R_MAX_VSIZE`. Fallback: Q4's second variant. |
| 6 | The new column climbs slowly, or the two runtimes drift. | The ladder gives a number from the first week; fixes to the old runtime need tests; the differential builtin harness. Two numbers a week: `passing-rt` and the builtin modules done. |
| 7 | The retarget: packages pinned today do not build against 4.6.2 headers. | Re-pin and re-run the census before freezing layouts (§9.2). If the lead keeps 4.5-era releases, the hidden entry points come back as a legacy shim over a side table, compiled only on request. |
| 8 | Compiling run-once code costs more than interpreting it. | Q9 and E9. |
| 9 | C frames as pins break a package that works on GNU R by luck. | Experiment A. Fallback: strong roots for C segments. |
| 10 | GNU R's counts copy where `Rc` did not, and a benchmark gets slower. | `perf.py` at the flip, by benchmark. The fix is porting the cleanup at call exit exactly, never extra decrements. |
| 11 | Interning makes string building slower than today. | Q7. If so, measure GNU R's uncached CHARSXP path for intermediates before accepting. |
| 12 | A weak cache word is missed, and a cache keeps an object alive. | The rule of §7.3 is a type: `CodeData` can hold `Strong<Code>`, `Strong<Shape>`, `Sym` or `WeakWord`, nothing else. Finalizer tests with R6 objects called through cached sites. |
| 13 | The 24-byte header breaks an alignment assumption or is not faster. | Q10; the package suites. Fallback: keep the word. |
| 14 | A package's ALTREP method evaluates R code under `Dataptr`. | GNU R runs `Dataptr` with collection inhibited; robigo does the same and raises a clear error on `Rf_eval` under it; vroom, arrow and cli find it. |
| 15 | Static immortals and isolates: a symbol's value and cache words are mutable. | Per-isolate arrays indexed by symbol id when isolates become threads. Not G3. |
| 16 | wasm32: the value stack reservation, 32-bit key bits. | The wasm32 build in CI from the prototype on. |
| 17 | The JIT lags and the flip waits. | The VM alone must pass E1 and E2; the JIT is gated per benchmark at step 5. |

Open questions, with my answer:

- **Should C frames stay strong roots regardless of experiment A, for
  safety?** No. Precise finalization is GNU R's semantics; the pin
  already gives more memory safety than GNU R has. Keep strong roots only
  if a package needs them.
- **Should the heap become generational before the flip?** No. Measure
  young-collection cost with frame recycling first; the store barrier is
  already in place.
- **Should hashed environments get shapes?** Not now (§3.2).
- **Should the prototype be throwaway?** No. It is the first commits of
  the real crates, so its numbers are numbers about the code that ships.

---

#### Appendix A: changes in `robigo-heap` relative to `robigo-core`

| area | change |
|---|---|
| layout (`lib.rs:155-181`) | `OFF_DATA` 24, `OFF_TRUELEN` removed; CHARSXP word `+12` symbol id; flags `WRITING`, `ESCAPED`, `SLOTS`; slot and key constants; private types 26 (Shape) and 27 (Code) |
| allocation (`gc.rs:659-700`) | no `collect_if_due` in any allocation path: set `poll`; `alloc_env(shape, enclos)`; `free_now` for recycled frames; the symbol arena (64 bytes, 64-byte aligned) |
| collection (`gc.rs:1524-1577`) | entered only from `Cx::safepoint` and export entry; C-segment scan after weak processing; roots: value stack by liveness map, context stack, the isolate's `Trace` impl |
| trace (`gc.rs:1349-1354`) | ENVSXP by flag: slots (`vtype == 0`) or node words; PROMSXP skips `+16` under `PROMISE_TAG`; typed binding cells; Shape and Code; weak cache words cleared after marking |
| static objects | the barrier's slow path and `incref`/`decref` accept objects outside any block |
| removed | HOST, `HOST_OWNED`, TWIN, WATCHED, `hold`, `watch`, `held_roots`, `host_trace`, the `env` shims (`lib.rs:791-848`), the Rust-side scan segments |

#### Appendix B: the porting contract after the flip (`AGENTS.md`)

1. GNU R 4.6.2's C sources are the spec. Port the C function. Mark ports
   with `// R: file.c:function`.
2. Base R's R code runs on top. Implement the internal.
3. Messages are verbatim. `errorcall` is `cx.error_call(call, msg)`;
   `error` is `cx.error(msg)`; warnings are `cx.warning_call` and
   `cx.warning` and are signalled where the C signals them.
4. Internal generics dispatch first.
5. **Copy-on-modify is GNU R's.** Test `maybe_shared` where the C tests
   `MAYBE_SHARED`, and duplicate where it duplicates. Do not add or
   remove a copy.
6. **References and roots.** A `Ref` lives until the next `&mut Cx`
   call. What must survive one is an argument or a root. A `PROTECT`
   across an `eval` or a warning in the C is a root in the port; a
   `PROTECT` across allocation only is nothing.
7. **Builtins contain no `unsafe`.** If the API cannot express
   something, the API changes.
8. No placeholders that guess: raise `robigo: '<feature>' is not
   implemented yet`.
9. Isolate state is a traced field of `Cx`. No `thread_local!`, no
   `static mut`.


### Opus

### G3, round 2: robigo's runtime rebuilt for the end state (Claude Opus 5.5)

Base: main `4cff0ceb`. Read-only. Inputs: `brief.md`, `brief-r2.md`,
`synthesis-r1.md`, all four round-1 designs (`astra.md`, `fable.md`,
`opus.md`, `muse.md`), `AGENTS.md`, `docs/core.md`, `docs/ledger/{perf-vm,
memory,core}.md`, `crates/robigo-core/src/lib.rs`, GNU R 4.5.0 and 4.6.2
sources (4.6.2 under the council scratchpad, `R-4.6.2/`). Paths starting
`R-4.6.2/` are in that tree. Guesses are marked *(guess)*. No builds or
measurements were run; every number I propose is an acceptance bar, not a
result.

---

#### Part I. Round-2 answers

##### I.1 What changed from my round 1, and why

1. **No Rust frame is ever scanned, and Rust allocation never collects.**
   In round 1 the collector scanned the innermost Rust segment
   conservatively, so that allocation could take `&Cx`. Now collection
   happens only at *safepoints*: functions that take `&mut Cx` (R entry,
   C entry, VM back-edges and calls, builtin return, explicit `gc()`) and
   allocations made by C code. Between safepoints Rust allocates freely,
   and `Ref<'g>` branding guarantees that no unrooted reference survives
   a safepoint. Rooting becomes precise and fully deterministic, with no
   `PROTECT` tax on Rust code. GNU R has the same idea: it runs finalizers
   only "at points where running arbitrary code should be safe"
   (`R-4.6.2/src/main/eval.c:1093-1096`, `:6244-6247`). Why the change: the
   lead's framing asks for the strongest guarantee. The round-1 hybrid
   still depended on LLVM not hiding pointers in derived forms (Muse's and
   Fable's objection, which I accept). It also left allocation-triggered
   finalizer timing nondeterministic and could not run on wasm32.
2. **C segments: conservative words only pin.** I adopt Fable's two-pass
   mark for the C frames that remain conservative. Finalizers and weak
   references are decided on precise marks alone. If experiment A
   passes, the naked `.Call` stub and the 1 KiB zeroing go.
3. **Warnings are immediate.** I drop my deferred-warning rule. A warning
   takes `&mut Cx`, as it does in GNU R (it can run handlers). The kernels
   that matter already warn after their loop in GNU R's C (`naflag` in
   `R-4.6.2/src/main/arithmetic.c:289-313`, `CoercionWarning` after the
   loop at `coerce.c:447`), so a verbatim port puts every warning where
   GNU R does.
4. **C-first builtins.** The new runtime takes every `.Internal` and
   `.Primitive` that the evaluator does not own from **GNU R 4.6.2's own C
   source**, compiled against robigo's `Defn.h`. This is how base-package
   natives already work (`docs/core.md` §8: stats natives from C gave
   fastr 30,576 vs 30,311 cases and examples 571 vs 545, with 0
   regressions). Rust implements the evaluator, and builtins only where a
   measurement shows that Rust is faster. In round 1 I re-ported all
   ~60k lines from C to Rust. This is possible only after G3: with one
   heap, GNU R's C runs on robigo's objects directly.
5. **The boot crutch is gone.** Round 1 had a leaf reverse bridge for
   booting. C-first boots base on day one with the full builtin set, so
   the boot cliff (my round-1 risk R1) disappears without any conversion
   layer.
6. **Target 4.6.2 frees the layout.** R 4.6 hides `TRUELENGTH`,
   `SETLENGTH` and `SET_TRUELENGTH` from packages (`R-4.6.2/src/main/
   memory.c:4085-4100`, `attribute_hidden`), adds the resizable-vector API
   (`Rinternals.h:1211-1215`), undeclares `FRAME`/`ENCLOS`/`HASHTAB` and
   adds `R_ParentEnv` (`memory.c:4684-4692`). It keeps `R_findVarLocInFrame`
   in `Defn.h` only (`Defn.h:2052-2059`), and adds the binding and dots
   APIs (`Rinternals.h:646-699`). Consequences: ENCLOS needs no fixed
   offset, `truelength` can leave the vector header (measured, §6), and
   caller-forced argument slots can answer the new binding API without
   ever making a PROMSXP.
7. **General environments use GNU R's representation verbatim** (Fable's
   design: FRAME pairlist of binding cells, HASHTAB chains). Round 1
   emulated GNU R's hash order over my own slot block. Slot frames stay
   for closure calls.
8. **One evaluator.** The AST interpreter is deleted. Every evaluation
   runs in the VM, with two compile profiles: one reproduces `eval.c`'s
   observable behaviour, the other the byte-code compiler's (§7.1).
9. **Unboxed promise values are GNU R's own.** GNU R 4.6.2 on 64-bit
   builds defines `IMMEDIATE_PROMISE_VALUES` (`Defn.h:34-39`): PRVALUE
   holds raw scalar bits with `PROMISE_TAG` set (`Defn.h:1269-1275`). I
   port that representation instead of inventing a 4th-word scheme.
10. **Contexts are GNU R's linked `RCNTXT` chain.** They are not a `Vec`.
    C-first builtins put `RCNTXT` records on their C stacks
    (`begincontext`), so contexts must be linkable records at stable
    addresses.
11. **The old runtime freezes at the start.** It gets no more fixes, only
    CI upkeep. All conformance work goes to the new runtime. Round 1
    allowed fixes that came with a test, and that let the two runtimes
    diverge.

##### I.2 Positions

**Dispute 1, handle.** `Ref<'g>`, branded by a shared borrow of the
isolate. It is stronger than in round 1, because branding is now the whole
rooting story. Allocation takes `&Cx` and never collects. Everything that
can collect or run R or C code takes `&mut Cx`, which ends every `'g`. A
value that must outlive a safepoint is in a precise root (argument
window, `Out` slot, `RootScope`, VM slot, frame, context or `Global`).
Answers to the round-1 objections:
- Fable and Muse said a brand infects 115k lines. Under plan B nothing is
  converted: the new runtime is written against the brand from its first
  line, and the long tail of builtins is GNU R's C, which needs no brand.
- Muse said brands fail at heap storage, VM registers and caches. Heap
  fields are traced object fields, not Rust values. VM registers are raw
  slots in the unsafe core. Caches are weak words in code objects (§7.4).
  The brand covers only Rust locals, which is where use-after-free comes
  from.
- Astra wanted scope brands, with every child read rooted. That is
  unnecessary once allocation cannot collect: a child read is valid until
  the next safepoint, exactly like its parent.

**Dispute 2, rooting.** Precise in Rust, with collection only at
safepoints. No Rust frame is ever scanned. C frames are scanned
conservatively but only pin memory (Fable's two-pass mark): the protect
stack and the precise roots decide reachability, weak references and
finalizers. GC timing is then a function of the program alone. This beats
every round-1 option: Astra and Muse's precise roots without the handle
traffic, Fable's memory safety without relying on pins in Rust, and my own
hybrid without the LLVM caveat.

**Dispute 3, unboxed values.** Unboxed in three places, all of them
measured:
- **VM temporaries**: tagged slots.
- **Bindings in slot frames**: typed 16-byte slots (Fable's key-with-kind
  encoding), GNU R's `BNDCELL_TAG` idea (`Defn.h:464`) inside the frame
  object.
- **Caller-forced arguments**: kept as eager slots, never as a PROMSXP.
  They are materialized for the legacy `findVarInFrame` path only. The
  4.6 binding API (`R_GetBindingType` → `R_BindingTypeForced`,
  `R_ForcedBindingExpression`) is answered from the slot and the call.

Heap PROMSXPs store unboxed values exactly as GNU R 4.6.2 does
(`PROMISE_TAG`). Muse's "registers only" leaves the frame boxing on the
general call, which is where robigo loses (`perf-vm.md:34-46`).

**Dispute 4, migration.** Plan B, sharpened. A new crate, `robigo-rt`,
lives on main and boots on GNU R's C builtins from day one. It has its
own ratchet column. The old runtime is frozen. At the flip the old
runtime and the bridge are deleted. Details in §9.

**Coordinator's pick 1 (plan B): agree**, with three changes: C-first
instead of re-porting from C to Rust; the oracle moves to 4.6.2 before
`robigo-rt` starts; the old runtime is frozen, not maintained in parallel.
**Pick 2 (branding): agree**, and add "Rust allocation never collects",
which is what makes branding sufficient. **Pick 3 (no deferred warnings):
agree**, for the reason in I.1 item 3. **Pick 4 (every variant built,
fastest exact wins): agree, with two conditions.** Variants are built at
full fidelity on the path being measured, not as toys. Variants that
interact (vector header × frame layout × slot encoding) are measured as a
small factorial; the others one at a time. Losing variants are deleted
when their decision is recorded, so no permanent `cfg` survives the
prototype.

##### I.3 The boldest changes now proposed

| # | change | end-state benefit | measurement that proves it | what it deletes |
|---|---|---|---|---|
| B1 | **Safepoint-only collection, branded refs, no Rust stack scan** (§2) | precise, deterministic GC everywhere; no `PROTECT` in Rust; wasm32 viable; a moving collector becomes possible | finalizer stress script (core ledger 2026-11-11), 1,000 ASLR runs, 0 misses at every depth and tier; gctorture-at-every-safepoint clean; `max_debt` (bytes allocated between safepoints) across all corpora below `R_VSize`; poll cost ≤ 1% Ir on fib and `calls.R` | Rust scan segments, `Core::scan_top` for Rust, my round-1 `RefVec` and `no_heap_refs.rs`, the stale-word bug class |
| B2 | **Pin-only C segments** (Fable's two-pass mark) | finalizers deterministic even across package C frames | experiment A (§10.1) on main today | `robigo_call65_stub`, the 1 KiB zeroing, segment-boundary narrowing (+108 instructions per `.Call`, core.md §3) |
| B3 | **C-first builtins**: GNU R 4.6.2's `src/main` C (108 files, 142,394 lines; ~40 of them evaluator-owned) compiled against robigo's `Defn.h`; Rust only for the evaluator and a measured hot set | exact by construction for the long tail (~600 internals); `robigo-rt` boots on day one; conformance at the flip ≥ the old runtime's | per-file A/B: ratchet delta and `perf.py` delta, C version vs old Rust port; cost of a C builtin call from the VM (trampoline) | ~40-50k lines of Rust builtin ports outside the hot set *(guess at the split)*; capi's duplicate semantics (`attrib.rs`, `coerce.rs`, `envir.rs`, …) |
| B4 | **One evaluator**: the VM runs everything, with an `Eval` profile (eval.c semantics) and a `Bc` profile (compiler semantics); code is cached in the LANGSXP/CLOSXP 4th word | one semantics instead of two; deopt target and baseline are the same | base boot time ≤ today; rlang and dplyr package-suite time ≤ today; compile Ir per AST node | the AST interpreter (`interp.rs` eval paths), interpreter-vs-VM disagreement as a bug class |
| B5 | **24-byte vector header** (no `truelength` word unless GROWABLE) | length-1 vectors in 32 bytes (today 48, GNU R 56); CHARSXPs smaller | bytes per call on `calls.R`, Ir and RSS on binarytrees, sapply, strings, package suites | the `truelength` word of every vector |
| B6 | **Debug moving collector, then a nursery bake-off**: `ROBIGO_GC_MOVE=1` evacuates every unpinned object at every collection and poisons the old copy; later a copying nursery with exposure pinning competes with sticky marks | any pointer held across a safepoint in unsafe code fails deterministically; then possibly cheaper young collections | the debug mode is a CI job; the nursery runs on the bake-off workloads + package suites (pauses, RSS, Ir) | nothing at first; the losing young-generation design |
| B7 | **Feedback-directed optimizing tier**: per-op type profiles in the code object; Cranelift specializes speculatively and deopts to the VM at op granularity | the path to "fastest R" on code the static inference cannot type | `perf.py` geomean and warm-up cost per benchmark | the static-only `JType` inference as the sole typing source |
| B8 | **Crates follow GNU R's files**: `robigo-rt/src/main/<file>.rs` holds the Rust port of `<file>.c`, both its `do_*` internals and its `Rf_*` API functions; `robigo-cbase` builds GNU R's C | one implementation per GNU R function; navigation by GNU R file name | count of functions implemented twice = 0 (script) | capi's parallel semantics; the "runtime vs capi" split of attributes, coercion, environments |

##### I.4 Taken from the others, and the one I think is wrong

**Fable.** Adopted: the two-pass mark; `ROBIGO_NO_INPLACE=1` as a
differential oracle for the link protocol; 16-byte typed slots with kind
bits in the key, which needs 64-byte-aligned symbols; general
environments in GNU R's representation verbatim (deletes `env/table.rs`);
S3 dispatch cached on the identity of the class attribute; one-word
`RResult` with the exit in the isolate; frame recycling under an ESCAPED
bit (measured). **Wrong:** "one API, two backends". An API that must also
compile over `Rc` cannot express branding or REFCNT semantics. Fable's
own text concedes that `keep` is a no-op under `Rc`, so backend A cannot
verify the rooting that converted code needs. The 85k lines would be
converted to an API whose hardest obligations are untested until the
flip.

**Astra.** Adopted: precise roots are not enough on their own. A
registered but dead root delays a finalizer as surely as a stale stack
word, so VM slots are cleared at last use (liveness at safepoints).
Context records at stable addresses. C/Rust `size_of`/`offset_of`
assertions for every `#[repr(C)]` record that C sees. "No cache keyed on
a bare address". **Wrong:** scope-branded `Value<'s>` with every child
read promoted into a scope root. That costs a root store per field read
to protect against collections that, in this design, cannot happen
between safepoints.

**Muse.** Adopted: builders, i.e. an unchecked fill path for unpublished
fresh objects (refcount increments only, no barrier and no old-value
decrement); a debug epoch on views that catches safepoints smuggled
through interior mutability; `Length` must never force an ALTREP's
elements. **Wrong:** the lazily rebuilt `RCNTXT` mirror. With GNU R's C
builtins on the hot path, contexts are pushed and read by C constantly,
so a mirror rebuilt at every native entry is the slow path made common.
There must be one chain.

---

#### Part II. The design

##### Crate map (end state)

| crate | contents |
|---|---|
| `robigo-core` | layout constants, heap, collector, CHARSXP cache, symbol arena, unwind trampolines. Unchanged role, new tracing cases (§2.5) |
| `robigo-rt` | `Cx`, the object API, environments, the compiler and VM, contexts, conditions, S3/S4 dispatch, the Rust builtins of the hot set, all C API exports (`src/main/*.rs` named after GNU R's files) |
| `robigo-cbase` | GNU R 4.6.2's C sources (`src/main` long tail, `src/appl`, `src/nmath` where measured better, base packages' `src/library/*/src`), compiled against robigo's headers by `build.rs` |
| `robigo-jit` | Cranelift tiers on the ABI of §7.5 (separate crate for compile time) |
| `robigo-capi` | shrinks to the headers, `Makeconf`, `dlopen`/registration, the graphics engine glue |
| `robigo-syntax`, `robigo-numerics`, `robigo-text`, `robigo-cli` | as today |

`robigo-runtime` and `robigo-capi/src/bridge.rs` (5,971 lines) are deleted
at the flip.

---

##### 1. The Rust-side object API

###### 1.1 Three layers

| layer | type | who | safety |
|---|---|---|---|
| raw | `robigo_core::Sexp` (`lib.rs:140`) | core, the VM and compiler (`rt/src/vm`), C exports | `unsafe`; core.md §3 rules plus the safepoint invariant (§2.2) |
| branded | `Ref<'g>`, `View<'g>`, typed views | Rust builtins, printing, deparse, coercion, attributes | safe; the borrow checker enforces rooting |
| rooted | `Root<'s>`, `Out<'s>`, `Args<'a>`, `Global` | anything held across a safepoint | safe; precise roots |

###### 1.2 Types

```rust
// crates/robigo-rt/src/obj/mod.rs
/// The isolate (§8). !Send + !Sync.
/// `&Cx`: read, allocate (never collects), store into objects, raise errors.
/// `&mut Cx`: anything that may collect or run R or C code.
pub struct Cx { /* §8 */ }

###[derive(Clone, Copy, PartialEq, Eq)]
###[repr(transparent)]
pub struct Ref<'g> { p: NonNull<u8>, _g: PhantomData<&'g Cx> }

/// Symbols are immortal: unbranded, 'static, 64-byte aligned (§3.6).
###[derive(Clone, Copy, PartialEq, Eq, Hash)]
###[repr(transparent)]
pub struct Sym(NonNull<u8>);

pub struct RootScope<'s> { base: u32, _s: PhantomData<&'s mut ()> } // truncates the protect stack on drop
###[derive(Clone, Copy)] pub struct Root<'s> { idx: u32, _s: PhantomData<&'s ()> }
pub struct Out<'s> { slot: *mut Slot, _s: PhantomData<&'s mut ()> } // a VM destination slot
pub struct Args<'a> { base: *const Slot, n: u32, tags: *const Option<Sym>, _a: PhantomData<&'a ()> }
pub struct Global(u32);                                              // isolate root table entry

impl Root<'_> { pub fn bind<'g>(self, cx: &'g Cx) -> Ref<'g>; }
impl Args<'_> { pub fn get<'g>(&self, cx: &'g Cx, i: usize) -> Ref<'g>; pub fn root(&self, i: usize) -> Root<'_>; }
```

`Ref<'g>::view(self) -> View<'g>` is the single match point. It costs
one byte load of the type at +0 and one of the flags at +1:

```rust
pub enum View<'g> {
    Null, Sym(Sym), Char(Char<'g>),
    Lgl(Vector<'g, Lgl>), Int(Vector<'g, i32>), Real(Vector<'g, f64>),
    Cplx(Vector<'g, Rcomplex>), Raw(Vector<'g, u8>), Str(StrVec<'g>),
    List(ListVec<'g>), Expr(ListVec<'g>),
    Alt(AltVec<'g>),                       // a vector of a *package* ALTREP class (Muse)
    Pairlist(Cons<'g>), Lang(Cons<'g>), Dots(Cons<'g>),
    Closure(Clo<'g>), Env(EnvRef<'g>), Promise(Prom<'g>),
    Builtin(Prim), Special(Prim),
    ExtPtr(ExtPtr<'g>), WeakRef(Ref<'g>), Bytecode(Ref<'g>), Obj(Ref<'g>),
}
```

Standard vectors and robigo's own ALTREP classes (compact sequences,
wrappers) appear as their typed view, because their methods are Rust and
run no R code. Package ALTREP classes appear as `Alt`, because their
methods are C, and C runs only under `&mut Cx`.

###### 1.3 The two kinds of operation

- **`&Cx`**: field reads (`car`, `cdr`, `tag`, `attrib`, `get_attr(sym)`,
  `len`, `enclos`, `formals`/`body`/`cloenv`, `prvalue`); allocation
  (`cx.alloc_real(n) -> FreshVec<'g, f64>`, `cx.cons`, `cx.mk_char`);
  stores through `Sexp::store` (barrier and `FIX_REFCNT`, `lib.rs:605`);
  CHARSXP interning; raising an error (§4.5). None of these collects:
  allocation only adds to the debt and sets the poll flag (§2.2).
- **`&mut Cx`**: `eval`, `apply`, `force`, `call_native` (any C, including
  `robigo-cbase`), `warningcall`/`warning`, signalling conditions, active
  binding reads and writes, package ALTREP methods, `safepoint()`,
  `collect()`, finalizers. Each may collect. The borrow ends every `'g`.

A builtin always gets `&mut Cx`. Inside it, `&Cx` stretches run between
safepoints:

```rust
pub type BuiltinFn =
    for<'a> fn(cx: &mut Cx, call: Root<'a>, args: Args<'a>, env: Root<'a>, out: Out<'a>) -> RResult<()>;

// R: seq.c:do_rep_len, as a sketch of the shape
fn do_rep_len(cx: &mut Cx, call: Root, args: Args, _env: Root, out: Out) -> RResult<()> {
    let n = as_len(&*cx, args.get(cx, 1), call)?;        // &Cx stretch: reads, may raise
    let x = args.get(cx, 0);
    let ans = cx.alloc_like(x, n)?;                       // allocates; cannot collect
    fill_recycled(x, &mut ans.as_mut_slice());
    out.set(ans.finish());                                // rooted by the VM slot
    Ok(())
}
```

`Args` is a window of the caller's VM stack, rooted for the whole call.
There is no `Vec<Arg>` per call. Today's signature allocates one
(`builtins/mod.rs:63`). A builtin that calls R code and needs its
intermediate results afterwards keeps them in a `RootScope`:

```rust
let s = cx.scope();
let acc = s.root(cx.alloc_list(n).finish());
for i in 0..n {
    cx.apply(f, &[args.root(0)], s.out())?;              // safepoint: only roots survive
    let v = s.last().bind(cx);
    acc.bind(cx).as_list().set(i, v);                    // Sexp::store
}
out.set(acc.bind(cx));
```

`RootScope` pushes onto the isolate's protect stack (`R_PPStack`, GNU R's
50,000 default and its overflow error). Each push is a store and an
increment. Contexts save and restore the top (core.md §5), so unwinding
needs no destructor (Muse's longjmp point holds).

A `Vec<Ref<'g>>` is allowed. It cannot live across a safepoint, because
the borrow checker kills it there. Round 1's heap-container hole does not
exist.

###### 1.4 Vector views and borrowed reads

```rust
impl<'g, T: Elt> Vector<'g, T> {
    pub fn len(self) -> usize;
    pub fn as_slice(self) -> &'g [T];          // inline, or a materialized own-class ALTREP
    pub fn elt(self, i: usize) -> T;           // INTEGER_ELT & co., own classes
    pub fn regions(self, buf: &mut [T; 512]) -> Regions<'g, '_, T>;   // ITERATE_BY_REGION, never materializes
}
impl Cx {
    pub fn alt_dataptr<T: Elt>(&mut self, x: Root) -> RResult<()>;   // package Dataptr, then re-view
    pub fn alt_regions<T: Elt>(&mut self, x: Root, f: impl FnMut(&[T]) -> RResult<()>) -> RResult<()>;
}
```

Soundness of `&'g [T]`:
- *GC*: nothing collects during `'g`, and the object does not move during
  `'g`, even under the debug moving collector of B6, which moves objects
  only at safepoints.
- *C writes through `REAL(x)`*: C runs only under `&mut Cx`, which ends
  `'g`. A pointer that a package keeps across calls (data.table) can be
  used only while C runs.
- *Rust writes*: only through a `FreshVec` (unique, `!Copy`) or through
  `cx.unique_mut(root)` (`&mut`, so no slice is alive). For a kernel that
  reads `y` while it writes `x` in place, use
  `cx.unique_mut2(x, y, |xs: &mut [f64], ys: &[f64]| ..)`, which copies
  when `x == y`.
- *ALTREP*: own classes are pure Rust. Package classes are `Alt` and need
  `&mut`. GNU R's C (C-first builtins) calls the methods exactly where
  GNU R calls them.
- *Debug*: under `debug_assertions` every view records `cx.epoch`, and
  every safepoint bumps it (Muse). A safepoint reached through interior
  mutability then fails an assertion.

###### 1.5 Copy-on-modify

- `refcnt` (+4, u16, saturating) is GNU R's `REFCNT`, maintained by
  `Sexp::store` (`lib.rs:605`). Bindings, list elements, attributes,
  cons fields and PRVALUE count. Locals, VM slots, `Args` and the protect
  stack do not.
- `MAYBE_SHARED` and `MAYBE_REFERENCED` are tested where GNU R's C tests
  them. In C-first builtins that holds automatically, because they are
  GNU R's C. In the Rust hot set they are ported from the same lines.
- **VM stack values**: GNU R's deferred link counting (`eval.c` around
  `R_BCProtCommitted` / `INCLNK_stack_commit`, 4.5.0 `eval.c:991-1050`).
  Before a mutating op (`SetIndex*`, a replacement call), the VM commits
  the counts of the object slots below the current activation and undoes
  them when the window pops. Functional code pays one compare.
- **Builders** (Muse): `FreshVec<'g, T>` fills `STRSXP`/`VECSXP` elements
  with increments only, and no barrier test or old-value decrement,
  because the object is unpublished and prefilled with nil
  (core.md §3). `finish()` publishes it.
- **Porting rule 5 of AGENTS.md ("take arguments by value") is
  retired.** Refcounts decide.
- `tracemem`: the `RTRACE` bit and duplicate.c's report. Addresses are
  real.
- **Verification**: `ROBIGO_NO_INPLACE=1` makes every `MAYBE_SHARED` true
  (Fable). Any output difference over the corpora between the two modes
  is a link-protocol bug. The other direction (copying where GNU R
  copies) is checked by the `tracemem` cases against the oracle.

###### 1.6 Constants

`R_NilValue`, `NA_STRING`, `R_BlankString`, the logical scalars,
`R_UnboundValue`, `R_MissingArg` and the ~300 predefined symbols are
static objects in the binary (Fable), so `x.is_null()` is a compare with
a link-time address. Their mutable parts (a symbol's base value, a global
cache cell) live in per-isolate tables (§3.6), which keeps them shareable
between isolates (core.md §10 A.2).

---

##### 2. Rooting: precise, collection only at safepoints

###### 2.1 What other systems do

| system | Rust/C++/C frames | lesson |
|---|---|---|
| GNU R | precise `PROTECT`; collects at any allocation; finalizers only at safe points (`eval.c:1093`, `memory.c:3091-3102`) | safe points for arbitrary code already exist in R |
| V8 / rusty_v8 | `HandleScope`, `Local<'s, T>` (`src/handles/handles.h`; rusty_v8 `src/scope.rs`) | precise, but every handle is a slot |
| SpiderMonkey | `Rooted<T>` plus a static hazard analysis (`js/src/devtools/rootAnalysis/`) | precise rooting needs a checker; here the borrow checker is the checker |
| JavaScriptCore | conservative machine stack (`heap/ConservativeRoots.cpp`) | works because JSC promises nothing about finalizer timing; R does |
| Julia | precise `JL_GC_PUSH` frames; `llvm-late-gc-lowering.cpp` places roots in compiled code | shows that compiled code needs a rooting contract |
| gc-arena / piccolo (`gc-arena/src/arena.rs`) | branded `Gc<'gc>`; collection only *between* mutation callbacks | **the model adopted here**, generalized to a stack-ful interpreter: branding plus collection only at `&mut` points |

###### 2.2 The rule

1. **Rust allocation never collects.** The allocator's slow path (block
   refill) updates GNU R's counters (`heapsize.rs`, R_NSize/R_VSize) and,
   when they cross the trigger, sets `cx.poll |= GC_DUE`.
2. **Safepoints** are the only places that collect:
   - R entries: `Cx::eval`, `apply_closure`, `force`, the VM's entry,
     `call_native`, the finalizer runner, `do_gc`;
   - the VM's back-edges and call ops (merged with today's interrupt
     poll, the analogue of GNU R's `bc_check_sigint`);
   - builtin return (the dispatcher, after `out` is written);
   - explicit `cx.safepoint()` in Rust code that wants to bound its
     garbage;
   - **every C API export that allocates**, at its entry (C-originated
     allocation keeps GNU R's "collect at any allocation" behaviour,
     because C is entered only through `&mut Cx`, so the Rust frames below
     it are suspended at a safepoint).

   A safepoint is `if cx.poll != 0 { cx.service()? }`: one load and one
   branch.
3. **Pending finalizers run where GNU R runs them**: at the VM poll (GNU R
   uses an eval count of 1,000 and its byte-code interrupt checks), after
   an explicit `gc()`, and after a native call returns.
4. **The unsafe core's invariant**: at a safepoint, every object the VM or
   compiler still needs is in a VM slot, a frame, a context, the protect
   stack or a `Global`. Dead slots are cleared (Astra): the compiler emits
   `Clear r` after the last use of a register that is live across a
   safepoint, or each safepoint has a liveness mask. Which is cheaper is
   decision D9.

What a collection marks, in order:
1. precise roots: protect stack, precious set, API globals, `Cx`'s global
   table, the context chain, handler and restart stacks, the VM stack
   (`[0, top)`, tag-checked), per-isolate symbol cells, `pending`;
2. weak references to a fixed point and finalizer readiness, **on these
   marks alone** (memory.c's `CheckFinalizers`);
3. **C segments** conservatively, as *pins*: memory reached only from
   there stays allocated, but it is dead for weak references and
   finalizers (Fable's reorder, local to `collect_needing`, `gc.rs:1524`);
4. the CHARSXP cache is pruned on the union of both mark sets, then
   sweep.

**Why it is deterministic.** No Rust frame is read, and conservative words
decide nothing about finalizers. Explicit `gc()` and allocation-triggered
collections depend only on the program. Allocation-triggered collections
fire at the first safepoint after the trigger, not at the allocation; GNU
R's timing is not reproduced exactly today either (`heapsize.rs`
approximates it).

**Why it is sound.** Safe code cannot hold a `Ref` across `&mut Cx`. The
unsafe core keeps rule 4, and two checks enforce it:
`ROBIGO_GCTORTURE=1` (collect at every safepoint and every C allocation,
poisoning freed slots) and `ROBIGO_GC_MOVE=1` (§2.6).

**Memory bound.** Garbage made inside one Rust stretch survives until the
stretch ends. Most builtins allocate only their result. A builtin that
allocates its result first, while only `Args` are live, uses
`cx.alloc_rooted(&mut self, ..) -> Root`, which collects first if a
collection is due. This keeps peak RSS on `numeric(1e9)`-style code at
GNU R's level. The heap records `max_debt` (the most bytes allocated
between two safepoints), and the CI corpora report it (risk R2).

###### 2.3 VM, JIT and LLVM

- VM slots are precise roots. The dispatch loop holds raw pointers in
  machine registers only between safepoints.
- Cranelift and later LLVM code follow one contract: **no object pointer
  and no pointer derived from one survives a safepoint in a machine
  register.** At a helper that can reach a safepoint, live objects are
  written back to their slots and reloaded afterwards. Derived data
  pointers are recomputed from the reloaded base. This is the contract
  that deopt already needs (write state back at op granularity), so
  there are no stack maps and no statepoints. It also makes a moving
  collector legal.
- Allocation from JIT code is a non-collecting fast path (bump plus a
  limit compare). Its slow path sets the poll flag and returns, and the
  next poll collects.

###### 2.4 C ↔ Rust ↔ C

Rust A runs `call_native` (`&mut`, safepoint) → C B (a segment opens) →
`Rf_allocVector` (an export: may collect; B's frames are pins, A's frames
are suspended at their safepoint) → `Rf_eval` (export → `Cx::eval`, R
entry) → Rust C (`&Cx` stretches between safepoints) → C D, and so on.
Every boundary is a safepoint and every Rust stretch starts with nothing
unrooted. Compiled GNU R C (`robigo-cbase`) is reachable from Rust only
through `Cx::c(&mut self)`, so a Rust stretch cannot reach a C function
that collects.

###### 2.5 What changes in robigo-core

- `collect_needing`: the two-pass order of §2.2. The Rust-segment kind of
  `ScanSegment` is deleted. C segments remain, as pins.
- New trace cases: slot-frame ENVSXP (§3.2), PROMSXP with `PROMISE_TAG`
  (skip +16), CODESXP (private type 27: constant pool, shape, weak cache
  block), the 64-byte symbol arena, the VM stack.
- Weak words (§7.4): registered blocks whose words are zeroed after
  marking when their target is unmarked.
- At the flip: HOST, `hold`, `watch`, `held_roots`, `host_trace`, TWIN,
  WATCHED and HOST_OWNED go (`gc.rs:976-1027`, `lib.rs:57-71`). The freed
  flag bits become `ESCAPED` (§4.3) and `PINNED` (§2.6).

###### 2.6 The debug moving collector (B6)

`ROBIGO_GC_MOVE=1`: at every collection, evacuate every object that is not
pinned into fresh memory, update every precise root and traced field, and
poison the old copy (an invalid type byte, then `0xDB` bytes). Pinned
objects stay: anything a C segment's words reach, anything ever returned
to C, and every CHARSXP. "Ever returned to C" means that every C API
export that returns a `SEXP` sets `PINNED`, and so does `.Call`'s
argument list. R 4.6's package API is functions only (no field macros,
`DATAPTR` undeclared, `VECTOR_PTR` removed, `NEWS` 593-708), so every
object a package sees passes through one of those points.
`DATAPTR_RO`/`STRING_PTR_RO` on a list pins its elements once and sets a
flag on the container. Base natives compiled with `Defn.h` macros are
built with `-DTESTING_WRITE_BARRIER` (core.md §8 already does this), so
their field reads go through the same functions.

In this mode a stale raw pointer in the unsafe core fails deterministically
at its next use, instead of only when a slot happens to be reused. It is a
CI job on the `rt` conformance slice. Production stays non-moving until
the nursery bake-off (§10.4) decides otherwise.

---

##### 3. Environments

###### 3.1 Two representations behind one ENVSXP

```text
ENVSXP           48 + 16*n bytes
+0   header      gp: FRAME_LOCK 1<<14, GLOBAL_FRAME 1<<15 (GNU R's ENVFLAGS)
                 flags: SLOTS (slot frame), ESCAPED (§4.3); extra: RDEBUG etc.
+8   attrib
+16  FRAME       pairlist of GNU R binding cells, or nil
+24  ENCLOS      (kept at +24 because it costs nothing; R 4.6 no longer requires it)
+32  HASHTAB     VECSXP of chains (GNU R's), or nil
+40  aux         slot frame: shape · general: version box (on demand) or nil
+48  slots[n]    slot frames only; n = LENGTH(shape)
```

**General frames** (global env, namespaces, package envs, `new.env()`,
`local()`, attached frames; frames that C creates): GNU R's
representation, with envir.c ported as written (`R_Newhashpjw`,
`R_HashSet`, `R_HashGet`, `R_HashResize`, prepending in unhashed frames).
`ls(sorted = FALSE)`, `as.list.environment`, `eapply` and `mget` then have
GNU R's order by construction. Binding flags are GNU R's cell bits
(`ACTIVE_BINDING_MASK` 1<<15, `BINDING_LOCK_MASK` 1<<14, MISSING, and the
typed cell tag in `extra`). This deletes `env/table.rs` (360 lines of
hash-order emulation).

**Slot frames** (closure calls, made by the VM from a compiled closure):

```text
slot (16 bytes):  key = symbol address (64-byte aligned) | bits
                    bit 0     LOCKED
                    bits 1-2  MISSING (GNU R's 0/1/2)
                    bits 3-5  kind: 0 object · 1 REAL · 2 INT · 3 LGL · 4 active (val = fun)
                              · 5 eager arg (object) · 6 eager REAL · 7 eager INT
                    bits 48-63 definition rank (for GNU R's listing order)
                  val = object, or the scalar's bits
```

- The shape is a VECSXP of symbols in slot order, owned by the closure's
  CODESXP. It lists the formals and the locals that the compiler finds
  (cmp.R's `findLocals` analysis).
- A variable the compiler did not see (`assign`, `eval` in the frame, C's
  `defineVar`) goes into FRAME as a GNU R cell. The frame is then
  "extended", and the fast paths test that one word.
- **Variables live in the frame, not in VM registers.** The VM reads and
  writes `env + 48 + 16k`. There is no register cache and no flush
  (deletes `Vars`/`flush_pending`, `vm.rs:301-366`), and `get()`,
  `sys.function` and C see current values. A scalar store writes the key
  (a constant per site) and the bits. An object store goes through
  `Sexp::store`.
- Reading a slot as an object boxes an unboxed scalar once and stores the
  box (`R_expand_binding_value`). This one function serves `get`,
  `as.list`, promises of the variable and C.
- The collector traces `val` for kinds 0, 4 and 5.
- `FRAME()` on a slot frame (still exported by 4.6.2, though undeclared,
  `memory.c:4684`) converts the frame **one way**: slots become GNU R
  cells in FRAME, the SLOTS flag clears, and the shape no longer applies.
  The frame's slot words then hold the cells (kind 0), so the VM stays
  correct after one re-check of the flag. No package in the census calls
  it (core.md §2) *(re-check against 4.6.2 builds)*.

Size: a three-binding call frame is 96 bytes in one allocation. Today's
`EnvData` alone is 240 bytes (`env/trace.rs:78`, Fable's count).

###### 3.2 Lookup and guards

- Own locals: slot index, fixed at compile time.
- Free variables in enclosing closure frames: a cache `{hops, shape, slot}`
  per `GetVar` site. Each hop checks shape and "not extended" (two loads,
  two compares). This is hidden-class caching; R makes it exact because
  every call of a closure has the same shape.
- General frames: the site caches the binding cell, as GNU R's byte code
  does. An unbound cell reads as `R_UnboundValue`, which is a miss.
- Global and search-path lookups: GNU R's `R_GlobalCache`
  (`envir.c:616-772`), one cell per symbol in the per-isolate symbol table
  (§3.6), flushed per symbol when a `GLOBAL_FRAME` frame changes.
- Function guards: a slot frame cannot bind a symbol outside its shape
  unless it is extended. This is known at compile time and costs one test
  at run time, which removes the "guard memo miss on every new frame" cost
  (~5,000 instructions per `calls.R` iteration, `perf-vm.md:72-75`).
  General frames get a version box on demand (a 16-byte RAWSXP in aux).
  A memo holds the box, never the frame's address.
- `<<-`: the same `{hops, shape, slot}` cache, then a slot write or a cell
  store. `x[i] <<- v` commits links first (§1.5).

###### 3.3 The 4.6 binding and dots API

`R_GetBindingType`, `R_MakeDelayedBinding`, `R_MakeForcedBinding`,
`R_MakeMissingBinding`, `R_ForcedBindingExpression`,
`R_DelayedBindingExpression`/`Environment`, `R_DotsLength`, `R_DotsElt`,
`R_GetDotType` and the rest (`Rinternals.h:646-699`) are implemented in
`rt/src/main/envir.rs` directly over both representations. An eager slot
answers "forced", with its expression taken from the call through the
call site's matching plan, and **no PROMSXP is made**. Only the legacy
paths that return the promise object (`Rf_findVarInFrame3`, `Rf_findVar`
on a promise binding) materialize it. Packages built against 4.6.2
headers are expected to move to the new API *(guess: rlang and vctrs
detect R ≥ 4.6)*, which makes materialization rarer still.
`R_findVarLocInFrame` is internal to `Defn.h` (`Defn.h:2052-2059`), so
`R_varloc_t` can be `(env, slot index)` for the base natives (methods)
that use it.

###### 3.4 Symbols

The symbol arena holds 64-byte, 64-byte-aligned, immortal symbols, shared
by isolates:

```text
SYMSXP  +16 PRINTNAME  +24 (unused; SYMVALUE is per isolate)  +32 INTERNAL
        +40 aux (nil)  +48 id u32  +52 hash u32  +56 spare
```

The CHARSXP keeps the symbol id + 1 in its spare word (+12), so
`install(CHARSXP)` is two loads with no hashing (Fable). The per-isolate
table `Cx.syms: Vec<SymCell>` (16 bytes per symbol) holds `base_value`
(GNU R's SYMVALUE for the base env and base namespace) and the
global-cache cell. Low symbol-address bits are free for slot keys.

---

##### 4. Closures, promises, matching, contexts, unwinding

###### 4.1 Layouts

| type | +16 | +24 | +32 | +40 |
|---|---|---|---|---|
| CLOSXP | FORMALS | BODY (always the AST) | CLOENV | CODESXP: compiled code for both profiles, inline info, shape, JIT entry |
| PROMSXP | PRVALUE: object, or raw scalar bits when `PROMISE_TAG` ≠ 0 (GNU R 4.6.2, `Defn.h:1269-1275`) | PRCODE | PRENV (nil once forced) | CODESXP of the thunk, or nil |
| LANGSXP | CAR | CDR | TAG | CODESXP compiled for `eval` of this call (`Eval` profile), or nil |
| CODESXP (type 27, private) | `*mut CodeData` (released by the sweeper) | constants VECSXP (traced) | shape | weak cache block |

- `PRSEEN` is gp bits 0-1, with GNU R's meaning, including the
  interrupted-promise warning (value 2, set by `R_restore_globals`).
- `SET_BODY`/`SET_FORMALS`/`SET_CLOENV` are hidden from packages in 4.6
  (`NEWS`); the internal setters clear aux.
- Code dies with its closure or call object. This deletes the
  never-evicting `CACHE` thread-local (`vm.rs:219`).

###### 4.2 A general call

`f(i * 0.5)` with `f <- function(x, scale = 1, label = "a")`:

1. Callee from the site's cache (weak word, guard per §3.2).
2. Matching: a memoized permutation per (site, callee formals object). The
   general path is match.c's three passes over the argument window.
3. One allocation: the slot frame, its keys prefilled from the shape.
4. Arguments: where today's analysis allows the caller to evaluate
   (`Code::strict`), the value goes straight into an eager slot (kind
   5-7), and no promise is made. Other arguments get a PROMSXP (one
   48-byte allocation). Constant defaults are bound as constants with
   MISSING set. Non-constant defaults get a promise.
5. A context record (§4.4), with no allocation. `promargs` is built only
   when `sys.function`, `match.call` or C asks for it.
6. The VM runs the body: one poll on entry.
7. Exit: GNU R's `R_CleanupEnvir` (4.5.0 `eval.c:2132`). If the frame
   escaped (ESCAPED set, or its refcount shows references from outside),
   eager slots become forced PROMSXPs before the call's data is gone.

Allocations per call: 1, plus 1 per lazy argument. GNU R: the env, a cons
per binding, a promise per argument, and the promargs pairlist.

###### 4.3 `...` and frame recycling

- `...` is a DOTSXP chain of promises (or of values carried in eager
  conses), bound in a slot. `..1`, `...length()`, `...elt()` and
  `list(...)` are VM ops. Forwarding `g(...)` splices the chain without
  copying it, as GNU R does.
- **Frame recycling** (measured, D2r): an unescaped, unreferenced frame
  goes onto a per-size LIFO list at exit. ESCAPED is set whenever the
  frame's address is handed out as a value with no counted edge:
  `environment()`, `sys.frame`, `parent.frame`, `sys.function`,
  `R_GetCurrentEnv`, `.External2`'s `rho`. Its competitor is ordinary
  allocation (bump allocation in mark-region already).

###### 4.4 The context chain

GNU R's `RCNTXT`, declared in robigo's `Defn.h`, laid out by `#[repr(C)]`
with `offset_of!` assertions against the C header (Astra):

```rust
###[repr(C)]
pub struct Rcntxt {
    nextcontext: *mut Rcntxt, callflag: i32, cstacktop: i32 /* R_PPStackTop */, // cjmpbuf: see below
    evaldepth: i32, _pad: i32,
    promargs: Sexp,             // nil until asked; built from args_base/nargs
    callfun: Sexp, sysparent: Sexp, call: Sexp, cloenv: Sexp, conexit: Sexp,
    cend: Option<unsafe extern "C" fn(*mut c_void)>, cenddata: *mut c_void,
    vmax: *mut c_void, intsusp: i32, gcenabled: i32, bcintactive: i32,
    bcbody: Sexp, bcpc: *mut c_void, relpc: isize,
    handlerstack: Sexp, restartstack: Sexp, prstack: *mut c_void,
    nodestack: *mut Slot, bcprottop: *mut Slot, bcframe: *mut c_void,
    srcref: Sexp, browserfinish: i32, returnValue: R_bcstack_t,
    jumptarget: *mut Rcntxt, jumpmask: i32,
    // robigo: where the args live, how to restore
    args_base: *mut Slot, nargs: u32, flags: u32, cjmpbuf: *mut JmpCtx,
}
```

- The field names are those of GNU R 4.6.2's RCNTXT (`Defn.h:1415-1448`),
  because C-first builtins (`print.c`, `deparse.c` and others) allocate
  RCNTXT on their own stacks, call `begincontext`/`endcontext` and set
  `cend`/`cenddata`. robigo's `Defn.h` declares the struct, so the order
  is robigo's. GNU R's inline `JMP_BUF cjmpbuf` becomes a pointer to the
  trampoline's jump context: the C files that `SETJMP` on a context
  (`context.c`, `errors.c`, `main.c`) are evaluator-owned and therefore
  Rust. A script checks that no C-first file touches `cjmpbuf`. The Rust evaluator allocates its records in a chunked
  arena, at stable addresses, linked through `nextcontext`.
- The chain is a precise root, as GNU R's `R_GlobalContext` is
  (memory.c).
- `sys.call`, `sys.function`, `parent.frame`, `sys.frames`,
  `on.exit(add, after)`, `returnValue`, restarts and browser state are
  ports of context.c over the chain.
- `R_HandlerStack` and `R_RestartStack` are GNU R's pairlists, per
  isolate, as errors.c expects.

###### 4.5 `RResult`, errors and C unwinding

```rust
pub struct Unwind;                        // 1-ZST: RResult<()> is a flag, RResult<Ref> one word
pub type RResult<T> = Result<T, Unwind>;
struct Pending { kind: ExitKind, target: *mut Rcntxt, mask: i32, value: Sexp, cond: Sexp, call: Sexp } // traced
```

- `Flow::error_call(call, msg)` works from `&Cx`: it records the message
  and call in `Pending` and returns `Unwind`. The dispatcher, which holds
  `&mut Cx`, builds the condition and runs calling handlers **before** any
  frame is popped. No R code can run between the `Err` and the dispatcher
  (R code needs `&mut`), so the order is GNU R's `errorcall` order.
- Warnings are `cx.warningcall(call, msg)` on `&mut Cx`, at GNU R's point
  (§I.1 item 3). A Rust kernel that warns roots what it still needs, as
  GNU R's C protects it.
- C ↔ Rust ↔ C is unchanged from core.md §5: trampolines, only export
  wrappers jump, `Err` becomes a `longjmp` to the nearest C context, and
  the trampoline turns the jump back into `Err`. C-first builtins raise
  through GNU R's own `errorcall`, which robigo implements, and that is the
  same path. The conversion layer and the `FLOWS` token table
  (`bridge.rs:88`) go.
- `tests/no_inner_jumps.rs` extends to `robigo-rt`.

---

##### 5. Strings

- `RString`/`RStr` (286 uses) disappear. A string element is a CHARSXP
  pointer (8 bytes, today 16). NA is the immortal `NA_STRING`.
- CHARSXP layout (with the B5 header):
  `+0 header · +8 hash u32 · +12 symbol id + 1 · +16 length · +24 bytes\0`.
  `TRUELENGTH` of CHARSXPs is hidden from packages in 4.6
  (`memory.c:4085`). For compiled GNU R C that still uses it as scratch
  (`radixsort.c`'s `savetl`), robigo's `Defn.h` maps it to a per-isolate
  side table that is valid only within the native call *(census of uses in
  the C-first set needed: `rg 'SET_TRUELENGTH' R-4.6.2/src/main`)*.
- Encoding bits are gp, with `robigo_core::chars` rules. `bytes() ->
  &'g [u8]` and `is_ascii()` are header reads. `utf8(&cx)` is
  `translateCharUTF8` (borrowed for ASCII and valid UTF-8), and
  `native(&cx)` is `translateChar`. Every GNU R translate point maps to
  one call. C-first string builtins (`character.c`, `grep.c`, `paste.c`,
  `sprintf.c`) use GNU R's own translation code directly, which removes
  the largest remaining class of encoding differences *(guess at the
  share)*.
- `match`/`unique`/`duplicated` hash the CHARSXP pointer when the
  encodings allow it (unique.c's `useCache`/`sequal` logic). This comes
  free when `unique.c` is compiled C, and a Rust port is kept only if it
  measures faster.
- `paste` builds into scratch and interns once per result (GNU R's
  structure). Interning costs 0.59x GNU R on a hit and 0.82x on a miss
  (core ledger 2026-10-28).

---

##### 6. Vectors and ALTREP

###### 6.1 Header and body (B5, decision D1)

```text
variant H24 (proposed):        +0 info(8) · +8 attrib · +16 length i64 · +24 payload
variant H32 (today):           +0 info(8) · +8 attrib · +16 length · +24 truelength · +32 payload
GROWABLE under H24:            truelength in the last 8 bytes of the allocation slot;
                               capacity = (slot size − 32) / elt size
```

`.Internal(inspect)` prints `tl=` (`R-4.6.2/src/main/inspect.c:139-141`):
0 for ordinary vectors, the stored value for growable ones, as GNU R does.
`R_allocResizableVector`, `R_resizeVector`, `R_maxLength` and
`R_duplicateAsResizable` (`memory.c:5051-5110`) are ported over that
word. `x[n+1] <- v` uses GNU R's `EnlargeVector` policy (GROWABLE,
`expand = 1.05`), so `tl` prints as in GNU R. Beyond GNU R's numbers,
robigo grows in place whenever the slot's size class has room, which
nothing in R can observe.

Expected gains: a length-1 vector takes 32 bytes instead of 48, and half
of all short vectors drop one size class. Bar for H24: ≥ 3% fewer
instructions or ≥ 5% less RSS on the allocation-heavy set (`calls.R`,
sapply-style, strings, package slices), and no regression above 0.5%
anywhere. Otherwise H32 stays. The constants live in `robigo_core`
(`OFF_DATA`), so the JIT and the headers follow by recompilation.

###### 6.2 Storage variants

| storage | representation | access |
|---|---|---|
| inline | payload at `OFF_DATA` | `as_slice` |
| own ALTREP | compact `intseq`/`realseq`, wrappers, deferred coercions (Rust classes) | `elt`, `regions`, `sum`, `is_sorted`, `no_na` from `&Cx`; `1:2e8` is one object (#460) |
| foreign ALTREP | Arrow, NumPy, mmap (core.md §2) | read through `Dataptr_or_null`; writable access materializes once |
| package ALTREP | `R_make_alt*_class` | `View::Alt`; methods under `&mut Cx`; called exactly where GNU R calls them |
| EXTDATA | only foreign writable buffers (`allocVector3` allocators) | as inline |

The bridge's EXTDATA arguments, the ALTREP copy registry and the deferred
wrappers (core.md §6, #422, #468) go. VM range registers become compact
`intseq` objects, one per loop.

###### 6.3 Attributes

A pairlist at +8, as in GNU R. `R_mapAttrib`, `R_getAttribCount`,
`R_hasAttrib`, `R_getAttributes`, `R_nrow`/`R_ncol` (4.6) are compiled
from `attrib.c` or ported, measured. One implementation serves R and C.
I considered a packed attribute array with a "shape" for dispatch caches
and rejected it: `ATTRIB()` must still return the authoritative pairlist
(C mutates it), and S3 caching on the class STRSXP's identity gets the
dispatch win without it.

---

##### 7. Compiler, VM and JIT

###### 7.1 One evaluator, two profiles (B4)

- Everything runs in the VM. `Cx::eval(expr, env)`:
  - a symbol is looked up and a constant returns itself (no compile);
  - a LANGSXP is compiled once with the **`Eval` profile** and cached in
    its 4th word.
- The `Eval` profile reproduces `eval.c`'s observables: no inlining of
  base functions, `R_Visible` per eval.c, error calls as eval.c reports
  them, and srcrefs per expression.
- Closures compile with the **`Bc` profile**, which reproduces the
  byte-code compiler's observables (inlined base functions, its
  `sys.call` and error-call behaviour). GNU R JIT-compiles closures
  before their first or second call and top-level loops, and evaluates
  other top-level code through eval.c. robigo picks the profile on the
  same rule, so output matches whichever tier GNU R would have used.
- Base bootstrap evaluates `rlib/base/R` through the `Eval` profile.
  Compilation must cost little on one-shot code, which bar E7 checks.
- This deletes the AST interpreter as an execution tier. Today the
  conformance runner compares interpreter, VM and VM+JIT; afterwards it
  compares only VM against VM+JIT, plus the GNU R oracle.

###### 7.2 The VM

- **Isolate VM stack**: a 256 MiB reservation committed on demand, with a
  guard page; a fixed array on wasm32. A frame is a window of slots.
- **Slot encoding** (decision D3):
  - *S16 (Fable)*: `{w0: object pointer, or a tag < 4096 (EMPTY, REAL,
    INT, LGL); w1: scalar bits}`. The GC scans w0 only.
  - *SoA (my round 1)*: `vals: [u64]`, `tags: [u8]`.

  There is no NaN-boxing: R's NA payload (`0x7FF00000000007A2`) must
  survive bit for bit.
- Registers hold temporaries only; variables are frame slots (§3.1).
- Dispatch (decision D10): a `match` loop with the general paths in
  `#[cold]` functions (the ledger's lesson), against a tail-call threaded
  interpreter if Rust's explicit tail calls (`become`) are stable by then
  *(guess at their status)*.
- **Type feedback**: each arithmetic, index and call op owns one feedback
  byte in `CodeData` (types seen, attributes seen, ALTREP seen) for the
  optimizing tier (B7).

###### 7.3 What carries over from perf-vm ("For G3")

Carried over: call-site function lookup through local frames (now shapes
and cells); pure-closure inlining; caller-forced promises (eager slots);
index promises forced in R's order; guard memoization (now shapes and
version boxes); static rejection memos; `%*%` row-vector dot products;
nested pure-call inlining; whole-closure native trees; the measurement
method (`BYFN=1 cmp.py`, `linediff.sh`, `asm.py`).

Redone: register buffers, `set_var`/`own_var`, `Vars`/`flush`, and the
layout of `run_with`.

###### 7.4 Inline caches

They live in the CODESXP. Objects go in the constant pool (traced) or in
**weak words**: the cache block at +40 is zeroed for unmarked targets
after marking. A strong cache would keep an R6 object alive through
`obj$method()` and delay its finalizer past `gc()`. That would be a
conformance bug, because GNU R has no such cache. JSC
(`finalizeUnconditionally`) and V8's weak feedback slots do the same.

| site | cache | valid while |
|---|---|---|
| call | callee CODESXP, matching plan, inline plan | callee aux unchanged, FORMALS same object |
| free variable | hops, shapes, slot | shapes match, frames not extended |
| global | binding cell | value not unbound |
| guard | version box, version | version unchanged |
| S3 dispatch | class STRSXP identity → method closure | same class object and method cell unchanged |

The S3 row targets the 18.8k instructions per dispatch measured today
(`perf-vm.md:80-82`).

###### 7.5 JIT ABI

```rust
type Entry = unsafe extern "C" fn(cx: *mut Cx, frame: *mut VmFrame) -> u32 /* exit code */;
###[repr(C)] struct VmFrame { slots: *mut Slot, env: Sexp, code: Sexp, pc: u32, .. }
```

- Header offsets come from `robigo_core` constants (`OFF_LEN`,
  `OFF_DATA`; slot-frame `OFF_SLOTS = 48`). A vector guard is two byte
  loads (`type`, `flags & (ALTREP|EXTDATA|OBJECT)`) and an attrib compare
  with nil's static address. The `VecView` marshalling (`jit.rs:121`) goes.
- Scalars stay in machine registers inside loops and are written back to
  frame slots at exits and safepoints. Slot writes are two stores (a
  constant key and the bits).
- In-place writes check `refcnt <= 1` at entry, after commit_links. Native
  code runs no R code between safepoints, so the check holds.
- Deopt: the frame state is already in slots, so the VM resumes at the
  uncommitted op. Call trees are replayed only when the purity rules
  prove nothing was observed (jit/closure.rs today).
- **Optimizing tier (B7)**: reads the feedback bytes, specializes, guards
  and deopts. It compiles after N back-edges or calls (tuned, measured).

---

##### 8. The isolate record

```rust
// crates/robigo-rt/src/cx.rs
###[repr(C)]
pub struct Cx {
    // hot prefix: fixed offsets, asserted, used by the VM and JIT
    vtop: *mut Slot, vlimit: *mut Slot, prot_committed: *mut Slot,
    ctx: *mut Rcntxt, pp_top: u32, eval_depth: u32,
    poll: u32,                        // INTERRUPT | GC_DUE | FINALIZERS
    visible: u8, _pad: [u8; 3],
    heap: robigo_core::Heap,          // bump pointer, limit, counters: alloc is two loads off cx
    pending: Pending,
    // everything else
    syms: Vec<SymCell>, globals: GlobalTable, handler_stack: Sexp, restart_stack: Sexp,
    search_epoch: u64, options, rng, conns, dlls, locale, numerics, console, jit, ..
}
```

- **What moves in**: all 69 runtime `thread_local!`s are gone with the
  old runtime. The new runtime has none: `scripts/lint.sh` fails on a
  `thread_local!` in rt, core or capi, except the one entry pointer.
  Most of today's (interner, `CACHE`, registries, `NEXT_ENV_ID`,
  epochs) have no successor, because objects carry their own data.
- **Rust**: `&mut Cx` and `&Cx` passed down. There is no TLS on any Rust
  path.
- **JIT**: `cx` is an argument kept in a callee-saved register.
- **C exports**: one TLS read at entry (`CURRENT: Cell<*mut Cx>`); in the
  executable that is one `%fs`-relative load. Decision D7 measures a
  sole-isolate global against it. Exported per-isolate data
  (`R_GlobalEnv`, `R_PPStackTop`, `R_Visible`, …) are macros over the
  accessor in robigo's headers (core.md §10).
- **C statics in `robigo-cbase`**: GNU R's `src/main` has ~460 file-scope
  `static` declarations by a rough count
  (`rg -c '^static [^(]*[;=]' R-4.6.2/src/main/*.c`); many are constant
  tables. For isolates as threads, the build rewrites each mutable one
  into a field of a per-isolate C struct reached through the same entry
  pointer. The rewrite is mechanical (clang AST tooling, checked into
  `scripts/cbase-isolate/`). Until it lands, `robigo-cbase` makes an
  isolate a process worker, as core.md §10 already prescribes for
  packages with C globals.
- **libR.so (item 15)**: after the flip the runtime makes no TLS calls
  besides the entry pointer, and the out-of-line allocator shim is the
  only measured blocker left. Re-run the 2026-11-10 experiment pair.

---

##### 9. Migration

###### 9.1 Why plan B, and what it fixes from round 1

The rejected alternatives are a mixed heap (unanimous), narrowing
`Value` on main and then a long swap branch (the target cannot be
expressed over `Rc`), and one API over two backends (rooting cannot be
verified under `Rc`). Plan B grows the target in its own crate on main,
gates it with its own ratchets, and flips when it contains the old
results. Round 1's risks were the boot cliff and divergence. C-first
removes the cliff: base boots on GNU R's builtins. Freezing the old
runtime removes the divergence.

###### 9.2 Steps

| step | where | what | exit |
|---|---|---|---|
| M0 | main | **Oracle to 4.6.2**: docker oracle, `rlib/` re-vendored from 4.6.2, every `.Rout` re-recorded at one commit; the old runtime's ratchets recomputed once against 4.6.2, with the delta recorded in a ledger (the only time markers may drop, because the expected outputs changed) | old runtime measured against 4.6.2: baseline numbers |
| M1 | main, `experiments/g3` and core | **prototype** (§10): `Cx`, object API, safepoints, slot and general frames, promises with eager slots, contexts, VM subset, `calls.R` unmodified; core changes (two-pass mark, trace cases, symbol arena, H24 variant, weak words) | exit criteria E1-E8 |
| M2 | main, `crates/robigo-rt` + `robigo-cbase` | evaluator core at full scope (both profiles, conditions, restarts, S3, browser hooks); `robigo-cbase` builds every non-evaluator `src/main` file; the builtin table maps each remaining `.Internal`/`.Primitive` to GNU R's `do_*` | `robigo-rt -e 1` boots base; conformance column `rt` starts with every case it passes |
| M3 | main | **the `rt` ratchet column**: `tests/r/passing-rt/`, `tests/corpora/*/passing-rt/`, `tests/capi/passing-rt/`; package suites in rt | rt ⊇ old on every corpus *(expected early, because the long tail is GNU R's C)* |
| M4 | main, rt | **the Rust hot set**: one PR per builtin family, each an A/B against its C version: ratchet-neutral and faster, or not merged. Start with what the VM touches: arithmetic, relop, logic, subset, subassign, `c`, `length`, `seq`, `rep`, `lapply`/`vapply`, `paste`, `match`, `unique` | `perf.py` per family |
| M5 | main, rt | C API exports over rt (capi's semantic files deleted); methods.so hooks into rt dispatch | capi corpus and package suites in rt ≥ old |
| M6 | main, `robigo-jit` | Cranelift on the §7.5 ABI: loops, then call trees, then the feedback tier | rt ≤ old on every JIT benchmark |
| M7 | main | **flip**: rt becomes `robigo`. One PR deletes `crates/robigo-runtime`, `bridge.rs`, core's HOST/hold/twin machinery and the scan stub (if experiment A passed), and moves `passing-rt` markers to `passing` | every old marker in rt's set; no benchmark slower than the old runtime by > 0.5%; G3 gate (design.md §5.6); issues #278 #277 #276 #189 #304 #167 #389 #217 #460 #140 closed with tests |
| M8 | main | nursery bake-off (§10.4), libR.so re-measure, isolate rewrite of `robigo-cbase` statics | ledger entries |

The bridge dies in M7 together with the old runtime, which needs it until
then.

###### 9.3 What pauses now, and what continues

- **Frozen**: the old runtime (CI upkeep only, no conformance fixes, no VM
  tuning, no bridge features); `RString`; Rust ports of base natives.
- **Continues**: robigo-syntax, robigo-numerics, robigo-text, robigo-core,
  the capi headers and loader, the package corpus, and new conformance
  tests. Every new test is automatically a target for rt.
- **Rewritten at M2**: AGENTS.md's porting contract. Rule 1 becomes "GNU
  R's C is the spec, and by default the implementation". Rule 5 is
  retired. "Writing a builtin" describes the branded API and the A/B rule.

###### 9.4 Who does what

- The evaluator core (`rt/src/main/{eval,envir,context,errors,objects,
  dotcode,memory-api}.rs`, `rt/src/vm`) has one owner per file
  (docs/workstreams.md).
- `robigo-cbase` build and headers: one owner.
- The hot set: many agents, one family each, each with its corpus filter
  in the `rt` column and `perf.py` as the metric.
- The old Rust ports are a reference for speed tricks, not a source to
  convert. A hot-set PR ports GNU R's C onto the branded API (it reads
  like the C: `car`, `cdr`, `view`, `alloc`, `MAYBE_SHARED`) and borrows
  the old port's kernels (x87 sums, radix order, `arith_exact.rs`) where
  they are representation-independent.

---

##### 10. Prototype and measurements

###### 10.1 Experiment A (on main now, no G3 code)

Reorder the mark so that conservative words only pin. Remove the naked
stub and the zeroing. Run the core and capi tests under gctorture, the
capi corpus, the finalizer stress script (`.Call('pointer')` at depths
0..20 × 0..20, 1,000 ASLR runs) and the CRAN package suites. Pass: no
regression and no missed finalizer. This decides B2.

###### 10.2 The prototype (`experiments/g3`, then promoted into `robigo-rt`)

Scope: parser output as LANGSXP; `Cx` and the branded API; safepoints;
slot and general frames; closures, promises with eager slots, `...`,
matching; the context chain with `on.exit`; conditions and restarts
loaded from `rlib/base/R` (`.addCondHands`, `.signalCondition`,
`.dfltStop`, `.addRestart`, `.invokeRestart`); the VM subset (variables,
`<-`, `<<-`, `if`, `for` over compact ranges, `while`, scalar and vector
arithmetic, `[[`, `[[<-`, `$`, `$<-`, calls with caches,
`UseMethod`/`NextMethod`); `gc()`, `reg.finalizer`, weak references;
**three `src/main` C files compiled from GNU R and called from the VM**
(`paste.c`, `unique.c`, `character.c`). The target: `calls.R` runs
unmodified, with the same output.

###### 10.3 Decisions

| id | question | variants | workload | metric |
|---|---|---|---|---|
| D1 | vector header | H24 vs H32 | calls.R, binarytrees, sapply, strings | Ir, bytes allocated, RSS |
| D2 | frame layout | slot frames (§3.1) vs GNU R pairlist frames vs env + separate slot vector | calls.R idioms, fib with JIT off | Ir, allocations per call |
| D2r | frame reuse | LIFO recycling with ESCAPED vs plain allocation | calls.R, fib | Ir, RSS |
| D3 | VM slot encoding | S16 vs SoA | fib, scalarFor, nbody | Ir, D1 misses (`--cache-sim=yes`) |
| D4 | heap promise values | `PROMISE_TAG` unboxed vs boxed | `f(i * 0.5)`, fib | Ir, bytes |
| D5 | caller-forced args | eager slots vs forced PROMSXP | calls.R, fib | Ir, allocations |
| D6 | link counting | GNU R's deferred commit vs eager | vectorFor, matrixFor, `l[[i]] <- x` | Ir |
| D7 | isolate access in exports | TLS vs sole-isolate global | `crates/robigo-capi/bench` | Ir per call |
| D8 | C builtin call cost | trampoline per call vs a no-setjmp fast path for C functions proven not to error (none in practice; measure the floor) | `nchar` in a loop through `character.c` | Ir per call; decides the hot-set threshold |
| D9 | dead-root clearing | `Clear r` ops vs liveness masks at safepoints | calls.R, finalizer script | Ir; 0 finalizer misses |
| D10 | dispatch | match loop vs tail-call threaded (if available) | the whole prototype set | Ir, wall |
| D11 | safepoint cost | poll at back-edges and calls vs at calls only plus a counter | fib, scalarFor | Ir |

Variants that interact (D1 × D2 × D3) are measured as a 2×3×2 factorial
on calls.R and fib; the rest one at a time. Losers are deleted when the
decision is recorded in `docs/ledger/g3.md`.

**Method**: callgrind instructions per call or iteration by N vs 2N
differencing (perf-vm's method), against today's robigo at equal tier
(JIT off, inlining off where the VM allows it) and against GNU R 4.6.2,
byte-compiled. Also: estimated cycles `Ir + 10·D1m + 100·LLm` (memory
ledger), `GcStats` allocations and bytes, peak RSS (`/usr/bin/time -v`),
p99 and maximum pause, and `max_debt`. Wall time with hyperfine
(30 runs, quiet machine) only for `calls.R`.

###### 10.4 Exit criteria (all must hold before M2 at scale)

- **E1**: `f(i * 0.5)` with two defaults left unforced costs ≤ 0.75x GNU R
  4.6.2's instructions and ≤ 0.6x today's 5,597 (`perf-vm.md:63`).
- **E2**: every `calls.R` idiom ≤ 1.0x GNU R's instructions with the JIT
  off (today 1.10-1.83x); `calls.R` wall time ≤ 0.8x GNU R's.
- **E3**: fib ≤ 0.9x GNU R's instructions with inlining and JIT off;
  binarytrees ≤ 0.8x GNU R's instructions, with peak RSS ≤ 1.1x GNU R's.
- **E4**: determinism. The finalizer stress script passes 1,000 ASLR runs
  with no miss at any depth. `ROBIGO_GCTORTURE=1` and `ROBIGO_GC_MOVE=1`
  are clean on every prototype test.
- **E5**: ergonomics. Three Rust builtin families (arithmetic,
  `assign`/`get`/`exists`, `lapply`) on the branded API, with **no
  `unsafe`**, reviewed by a council member who did not write them.
- **E6**: the three compiled C files pass their corpus slices in the
  prototype with output identical to GNU R; D8 measured.
- **E7**: compiling one-shot code costs little: loading `rlib/base/R`
  through the `Eval` profile ≤ 1.1x today's boot instructions.
- **E8**: `max_debt` ≤ 64 MB on every prototype test, and the capi corpus
  cases `eval`, `pairlist`, `unwind` and `extptr` pass with one heap and
  no bridge.

If E1-E3 fail, the decision table says which variant to try next. If E5
fails, the branded API is wrong and is fixed before M2. E5's failure would
not justify returning to unbranded values.

###### 10.5 After the flip: the young-generation bake-off (M8)

Three contestants, all behind the existing store barrier:
- sticky-mark non-moving generational (core.md §11 item 1);
- a copying nursery with exposure pinning (B6's machinery made
  production);
- MMTk StickyImmix (core.md §11 item 4). MMTk can use the precise Rust
  roots that the safepoint rule now provides; C segments become its
  pinning roots.

Workloads: the bake-off set and the package suites. Metrics: Ir,
estimated cycles, RSS, p99 and maximum pause, `mclapply` copy-on-write
RSS (core.md §10). The winner must beat mark-region on pauses without
losing Ir.

---

##### 11. Risks and open questions, ranked

| # | risk | why | what resolves it |
|---|---|---|---|
| R1 | The unsafe core holds a raw pointer across a safepoint | use after free | `ROBIGO_GC_MOVE=1` (deterministic fault on next use) and gctorture at every safepoint, both as CI jobs on the `rt` slices; the VM's safepoint list generated from one table |
| R2 | Garbage piles up inside long Rust stretches | RSS above GNU R's | `max_debt` reported by every corpus run; `alloc_rooted` for result-first builtins; explicit `cx.safepoint()` where `max_debt` exceeds the bar |
| R3 | C builtins cost more per call than Rust ones | regressions on hot paths | D8; the hot set is chosen by measurement (M4); flip gate: no benchmark slower |
| R4 | The link protocol misses a case | silent wrong answers | `ROBIGO_NO_INPLACE=1` differential oracle over all corpora; `tracemem` cases |
| R5 | Moving the oracle to 4.6.2 changes expected outputs and hides regressions | ratchet trust | M0 at one commit, the old runtime's delta recorded case by case in the ledger, and no other marker removals ever |
| R6 | C statics in `robigo-cbase` block isolates as threads | core.md §10 requirement | census and mechanical rewrite (§8); process workers until then |
| R7 | Eager slots or lazy promargs miss a reader | `substitute`/`match.call`/C differences | `ROBIGO_MATERIALIZE_TORTURE=1` (materialize everything eagerly; corpora must match in both modes); `tests/r/basics/caller-forced-promises.R` |
| R8 | The two compile profiles diverge from GNU R's tier choice | output differences in edge cases (error calls, visibility) | conformance cases that run the same code top-level, in a loop, in a closure and byte-compiled, all against the oracle |
| R9 | Slot frames' listing order | `ls(sorted = FALSE)` differences | the rank bits; the existing order tests in the rt column; convert to general on doubt |
| R10 | GC pauses at package scale | package suites already at only 0.7-1.0x GNU R's time | package suites in the rt column (RSS, pauses, `GcStats`); M8 bake-off |
| R11 | REFCNT copies where `Rc` did not | benchmark regressions | `perf.py` per family; the fix is porting `R_CleanupEnvir` exactly, never extra decrements |
| R12 | A package ALTREP method evaluates R code or needs GNU R's exact collection timing | crashes or differences | methods run under `&mut Cx` like any C; the package suites (vroom, arrow, cli) in rt |
| R13 | The JIT rewrite lags behind the flip | perf gate blocks M7 | VM alone must meet E1-E3; the loop tier ships before call trees |
| R14 | Symbol arena as one reserved range | wasm32 address space | fixed-size arena on wasm32; ids in the symbol itself (+48) need no address arithmetic |

**Open questions for the lead**

1. **C-first (B3)** changes robigo's identity from "Rust reimplementation"
   to "Rust evaluator, GNU R's C where it is as fast". I believe the
   contract (exact behaviour, measurement as arbiter) favours it, and the
   stats natives already set the precedent. The lead decides.
2. **H24 (B5)** changes the core layout that G2 fixed. It is reversible
   until the JIT and the headers ship against it. Decide at D1.
3. **The sole-isolate global (D7)** is process-global mutable state, which
   core.md §10 forbids in libR. It is a cache of the TLS value, kept only
   if D7 shows a gain.


### Muse

### G3 round 2: robigo rebuilt as `robigo-rt` on a reopened core, targeting GNU R 4.6.2

Muse Wonder Full, design council round 2. Base `4cff0ceb142dfa4547a840b1bfa4e1fa4b7360f4`
(verified by `git rev-parse HEAD` in this worktree). Read-only review of all
four round-1 designs (`astra.md`, `fable.md`, `opus.md`, `muse.md`),
`synthesis-r1.md`, `brief.md`, `brief-r2.md`, the repo at the base above, and
GNU R 4.6.2 at `…/scratchpad/r462/R-4.6.2` (`doc/NEWS` C-Level Facilities,
`src/include/Rinternals.h`, `src/main/{envir,eval,memory}.c`,
`src/include/Defn.h`). Citations are `path:line`; `R462/` prefixes the 4.6.2
tree, `R450/` the 4.5.0 tree where round 1 cited it. Guesses are marked
*(guess)*. This file stands alone.

#### 0. What changed from my round 1, and why

1. **Migration: I withdraw the ≤7-day swap branch and the dead-code `obj.rs`
   on main.** I adopt Opus plan B: a new `crates/robigo-rt` on main with its
   own ratchet column, builtins re-ported from GNU R's C, flip-and-delete.
   My round-1 argument against it was pure effort/divergence cost ("the same
   conversion work, plus an unbounded period… plus a second swap"), which
   the lead's framing explicitly voids. What remains is the correctness
   argument, and it runs the other way: a 7-day big-bang over ~115k lines
   has no green signal until base boots, so it is the *least* verifiable
   plan on the table. Plan B is hill-climbable from day 1.
2. **Handle: I withdraw plain `Copy Value` and adopt compile-time branding**
   (Opus's `Ref<'g>` tied to `&Cx`), with one amendment Opus needs and Astra
   identified: allocation must auto-root its result (§1.2), because I reject
   the conservative Rust scan that kept Opus's unrooted `Ref`s alive (§2).
   My round-1 objection — branding infects 115k lines of signatures — was an
   effort argument. Under "effort is free" the strongest static guarantee
   wins, and branding is it.
3. **Rooting: unchanged in direction (precise Rust, conservative C only),
   hardened.** Round 1 I left a hole: a bare `Value` local "valid until the
   next safepoint" was a comment-level rule. Branding + auto-rooted
   allocation + a precise value stack (§2) make it checked. I reject both
   rivals' relaxations: Opus's innermost-Rust-segment scan as *roots*, and
   Fable's pins-after-weak-processing (§2.1 gives the counterexamples).
4. **Deferred link counts: adopted.** The synthesis correctly flags this as
   my round-1 gap. `c(x, {x[1] <- 0; x})` must copy. GNU R's mechanism
   (`R462/src/main/eval.c:997-1016`, `R_BCProtCommitted`,
   `INCLNK_stack_commit`) is now §1.5, ported exactly.
5. **Environments: I withdraw the fixed 80-byte ENVSXP + slot VECSXP (two
   allocations per call) and adopt Fable/Opus variable-size ENVSXP with
   inline typed slots (one allocation, 48+16n).** Variables live in the
   environment, not in VM registers with a flush; my `Vars`-flush carry-over
   is deleted. GNU R's own typed binding cells
   (`R462/src/include/Defn.h:464`, `BNDCELL_TAG`) are the precedent.
6. **Promises: I withdraw "unboxed scalars never reach the heap."** Typed
   binding slots (eager arguments, §4.2) are the primary unboxed home; the
   PROMSXP aux-word scalar (Opus) is a measured prototype variant. Registers
   alone lose because C's `findVar` and R's `get()` must see current values
   with no write-back — Fable's decisive point.
7. **`Sym` becomes the SYMSXP pointer; the `u32` index is deleted.** My
   round-1 "deliberate non-churn decision" was effort reasoning. One symbol
   identity, dense id by arena arithmetic (§5), is the better end state.
8. **Target is GNU R 4.6.2, and the core is reopened.** Round 1 treated the
   48-byte node, the universal traced aux word, and the bake-off collector
   as settled. Round 2 puts all three back on the table with pre-registered
   measurements (§0.3, §10), and exploits what 4.6 hides from installed
   headers (§0.4).
9. **Warnings: no deferral, anywhere.** Round 1 did not propose deferral,
   but I now state it as a rule with a mechanism (§4.5) because Opus's
   deferred-warning design is the price his `&Cx`-allocation scheme pays,
   and I refuse to pay it.

#### 0.1 Position on the four disputes

| Dispute | My position |
|---|---|
| 1. Handle | **Opus branding, amended.** `Ref<'g>` branded by `&'g Cx`; allocation takes `&Cx` and auto-roots (§1.2); anything that runs R or C code takes `&mut Cx` and kills every live `Ref`. Astra's scope brand is stronger still but taxes every child read with a root and is bypassed by the VM anyway; Fable's plain `Copy` + runtime strict mode checks in CI what branding checks at compile time. |
| 2. Rooting | **Astra/muse-r1, hardened.** Precise roots decide reachability *and* finalizers for all Rust frames; conservative scan only inside C segments, as strong roots, exactly as today. Explicit `gc()` scans no Rust frames. Reject Opus (current-segment stale words still delay finalizers on allocation-triggered collections, and a register-only or derived-only pointer is invisible — unsound in the other direction). Reject Fable (a pin-only EXTPTR gets finalized while Rust/C still holds it: the finalizer frees the C resource under a live pointer — memory stays valid, the resource does not; that is worse than GNU R, which at least frees both together). |
| 3. Unboxed promise/binding values | **Fable typed binding slots as the design; Opus aux-word promise scalar as a measured variant.** Binding slots need no tracer change beyond the new ENVSXP case and serve eager arguments, locals and `<<-`. The promise aux variant needs the tracer to skip a word by gp tag (`crates/robigo-core/src/gc.rs:1349-1354` marks it unconditionally today); it is adopted only if prototype D4 (§10) shows ≥3% on the promise-heavy `calls.R` rows at zero conformance cost. Registers-only (my r1) is withdrawn. |
| 4. Migration | **Opus plan B (`robigo-rt`), with Fable's count-ratchet idea inverted and Astra's "two executables are not a mixed heap" as the framing.** Reject Fable's one-API-two-backends: an API that also runs over `Rc` cannot express branding, auto-rooting or typed slots, and `keep()` as a no-op under backend A means the rooting discipline is *never* exercised by the ratchets that supposedly verify the conversion. Reject my r1 short swap and Astra's coordinated branch: unverifiable until the end. |

#### 0.2 Position on the coordinator's four tentative picks

1. **Plan B migration — agree, with three amendments.** (a) The prototype is
   not throwaway: `crates/robigo-rt` *is* the prototype grown up, so its
   first commits are already on the ratchet column. (b) Divergence control is
   Opus's rule, made mechanical: after M1 (§9), an old-runtime fix merges
   only with a committed conformance test, and that test automatically
   enters the `rt` target set; a weekly `rt`-deficit count (old passing ∖ rt
   passing) is reported and may only shrink after M4 starts. (c) The boot
   crutch (leaf-only reverse bridge) is allowed only behind a single table
   in one file, with a ratchet that the table's length never grows after M3
   and is empty at flip; if the prototype shows base needs <150 internals,
   skip it entirely.
2. **Compile-time branding — agree, with the auto-root amendment (§1.2)**
   and with warnings excluded from the `&Cx` world entirely (pick 3). One
   more condition the coordinator's pick does not state: branding is only
   as strong as the ban on branded types in Rust heap containers, so that
   ban is a source-scanner test from the first `robigo-rt` commit (§1.3).
3. **No deferred warnings — agree unconditionally.** Every warning is
   signalled at the GNU R call site, through `&mut Cx`, with views dropped
   and re-acquired. The cost is real but it is paid on cold paths; the
   alternative makes message order, `warn=2` aborts and
   `withCallingHandlers` side effects depend on a "pure builtin" list
   maintained by hand. Verbatim messages are the contract; a hand-maintained
   purity list is exactly the kind of thing that rots.
4. **Every variant built in the prototype, fastest exact one wins — agree
   in spirit, bound in practice.** Unbounded variant-building is how a
   prototype becomes a second runtime. Each variant gets: one cargo feature
   in `robigo-rt`, one row in the §10 decision table with a pre-registered
   numeric decision rule, and a deletion commit for the loser *before* M3
   (evaluator completion) starts. Variants that cannot be A/B'd behind a
   feature in the same binary (collector replacement, §0.3 B3) are settled
   by the bake-off harness in `experiments/`, not in `robigo-rt`.

#### 0.3 The boldest changes round 1 held back on

Each item: end-state benefit, the measurement that proves it, what it
deletes. All are in scope because the lead voided effort/churn objections;
each still has to win on numbers or exactness.

- **B1. Retarget the whole project to GNU R 4.6.2 before G3 code lands.**
  Oracle, `rlib/`, expected outputs, `names.c` tables and the porting
  contract move 4.5.0 → 4.6.2 as their own campaign on main, on the *old*
  runtime, so G3 never has to distinguish "representation bug" from
  "version drift". Benefit: one moving target at a time; the 4.6 binding
  API (§0.4) becomes available to design against. Proof: full ratchet
  regeneration with the 4.6.2 oracle (`scripts/build-oracle.sh` pointed at
  4.6.2); every changed `.Rout` individually attributable to a `doc/NEWS`
  entry. Deletes: every 4.5.0-ism and the need to ever do this migration
  *during* G3. (If the lead's pending decision lands on staying at 4.5.0,
  B1 drops out and §0.4 becomes a forecast, nothing else changes.)
- **B2. Re-port the runtime from GNU R's C; do not convert the old Rust.**
  `robigo-rt` builtins are ported function-by-function from 4.6.2 C sources
  onto the branded API, with the old Rust port as a cross-reference only
  where it already diverges from C shape. Benefit: the code ends up shaped
  like the spec, which is what makes the *next* version bump (4.7, …) a
  diff-reading exercise instead of archaeology; C `PROTECT`s map 1:1 to
  rooting decisions the compiler then checks. Proof: per-module corpus
  slice green in the `rt` column + instruction count ≤ old module on its
  benchmark rows. Deletes: at flip, all of `crates/robigo-runtime`
  (~115k lines), `crates/robigo-capi/src/bridge.rs`, `host.rs`, and the
  codemod I proposed in round 1 (a codemod remains for the purely
  mechanical residue inside a re-port, §9.4, but it is no longer the plan).
- **B3. Reopen the heap: typed node sizes and a collector bake-off, round 2.**
  (a) Abolish the universal 48-byte node with traced aux. Node size becomes
  per-type: plain LISTSXP 40 B (header+car+cdr+tag, no aux), LANGSXP/
  CLOSXP/PROMSXP/DOTSXP 48 B (aux = code/cache), ENVSXP variable (§3),
  SYMSXP 64 B in its own arena (§5). The tracer already switches on type
  (`gc.rs:1306` `trace`, node arm at `:1349-1354`); per-type sizes are a
  size-class table change, not a new collector. (b) Re-run the memory
  bake-off with the *real* G3 workload (the `robigo-rt` prototype, not the
  bridge) over three collectors: current mark-region; mark-region + sticky
  generational young collections behind the existing `Sexp::store` barrier
  (`crates/robigo-core/src/lib.rs:605`); mark-region + LIFO frame/promise
  recycling with an ESCAPED bit (Fable §4.3). Benefit: the two costs G3
  actually pays — call-frame churn and package-suite pauses — get decided
  by the workload that pays them. Proof: `experiments/memory`-style
  harness, metrics Ir/alloc, p99 pause, peak RSS on binarytrees + `calls.R`
  + one package suite; decision rule pre-registered in §10 (D9). Deletes:
  whichever mechanisms lose; if recycling wins, most young-frame GC traffic;
  if 40 B cons wins, 17% of every pairlist's footprint.
- **B4. One execution IR; the tree-walking interpreter becomes a test
  oracle, not a tier.** Today robigo has AST eval, a register VM, a loop
  JIT and whole-closure JIT with subtly separate semantics to keep in
  lockstep. End state: the compiler lowers LANGSXP → one SSA-ish IR in
  `robigo-rt/src/compile.rs`; the VM executes that IR; Cranelift tiers
  compile the same IR; a *reference interpreter* over LANGSXP exists only
  behind `ROBIGO_REF_EVAL=1` for differential testing
  (`compiled_matches_interpreted` becomes a three-way byte-identity gate:
  ref vs VM vs JIT). Benefit: one semantics to port `eval.c` into, one
  place where promise/index forcing order lives. Proof: differential gate
  over the full conformance suite in all three modes; `calls.R` Ir/call.
  Deletes: `interp.rs`'s parallel evaluation paths (the new ref interpreter
  is a deliberately naive port of `eval.c:eval`, a fraction of the size),
  and every "AST tier does X, VM tier does X′" fix-up.
- **B5. Environments stop pretending to be GNU R pairlists internally —
  4.6 allows it (§0.4).** Slot frames are the only closure-frame
  representation; GNU R FRAME/HASHTAB pairlists are *materialized views*
  built by the warned-but-present C entry points, never the store.
  Hashed/user environments use robigo's own open-addressing table over
  slot storage, with GNU R's enumeration order computed from `order[]`
  ranks, not simulated buckets. Benefit: one frame representation, index
  addressing everywhere, no dual-bookkeeping. Proof: `ls(sorted=FALSE)` /
  `as.list` / `eapply` order conformance must be byte-identical (this is
  observable behaviour and non-negotiable); lookup Ir on `calls.R` and a
  10k-binding namespace microbenchmark. Deletes: `env/table.rs`'s GNU R
  hash-order simulation, the pairlist frame as a storage format, and my
  round-1 "general mode is GNU R bit-for-bit" — replaced by "general mode
  is GNU R *behaviour* bit-for-bit, storage is ours". Fable wants GNU R's
  chained HASHTAB kept for order-by-construction; I take the risk
  knowingly because order is fully testable and the table is on hot paths
  (namespace lookups) where chaining loses. If order conformance shows a
  single unexplained diff, B5's hashed half reverts to Fable's design —
  that fallback is decided here, in advance.
- **B6. The builtin registry is generated from GNU R's `names.c`, not
  hand-written.** A build-time generator (checked-in output, regenerated
  per R version) emits the `prim/special/internal` table with arity, eval
  and visibility columns read mechanically from
  `R462/src/main/names.c` (e.g. `{"gc", do_gc, 0, 11, 3, …}` at
  `names.c:700`). Hand `DEFS` tables remain only as the list of *which*
  functions are implemented in Rust vs base-native C. Benefit: the
  XYZ-column class of bug (wrong visibility, wrong eval) becomes
  impossible rather than reviewed-for. Proof: generator output diff
  against today's tables must be empty or each diff a filed bug; visibility
  conformance (`withVisible`) green. Deletes: hand-maintained arity/eval/
  visibility data in every `builtins/*.rs` `DEFS`.
- **B7. `libR.so` is the primary artifact; the executable is a thin main.**
  Round 1 (all four of us) treated this as a post-G3 measurement. Under
  the new framing it is a design constraint from the first `robigo-rt`
  commit: no new `thread_local!` (lint-enforced), one TLS read at C entry,
  isolate record (§8) as the only home of mutable state. Benefit: embedding
  (the actual GNU R distribution shape) stops being a retrofit. Proof:
  re-run the 2026-11-10 shared-library experiment at M7; accept only if
  the "no benchmark slower" rule holds, else the constraint still cost
  nothing measurable. Deletes: the 69 `thread_local!`s as a category.
- **B8. Verification modes as permanent product features.**
  `ROBIGO_NO_INPLACE=1` (every `MAYBE_SHARED` true — Fable's differential
  oracle for the link protocol), `ROBIGO_MATERIALIZE_TORTURE=1` (every
  eager slot/promise materializes eagerly — Opus R12), `ROBIGO_GC_STRICT=1`
  (gctorture + poisoned free slots + no C-segment scan widening) and
  `ROBIGO_REF_EVAL=1` (B4) run in CI forever, not as migration scaffolding.
  Benefit: the four scariest G3 mechanisms each get a semantics-preserving
  off-switch, so any future bug in them is bisectable by env var. Proof:
  the modes themselves — output must be byte-identical across all modes
  over conformance + corpora slices. Deletes: nothing; costs four `if`s on
  cold paths. This is the cheapest boldness on the list.

#### 0.4 What GNU R 4.6.2 frees up (and what it does not)

Verified against `R462/doc/NEWS` (C-Level Facilities) and
`R462/src/include/Rinternals.h`:

- **Hidden from installed headers** (no declaration, hidden symbol):
  `FRAME`, `HASHTAB`, `ENCLOS`, `ENVFLAGS`, `SET_FRAME`, `SET_HASHTAB`,
  `SET_ENCLOS`, `TRUELENGTH`, `SET_TRUELENGTH`, `XTRUELENGTH`, `LEVELS`,
  `NAMED`, `SET_NAMED`, `OBJECT`, `SETLENGTH`, `STDVEC_DATAPTR`,
  `SET_BODY`, `SET_CLOENV`, `SET_FORMALS`, `EXTPTR_PTR/PROT/TAG`,
  `SET_GROWABLE_BIT`, `IS_S4_OBJECT`/`SET_S4_OBJECT`, … Consequence:
  **package-visible C source compatibility no longer constrains ENVSXP
  layout at all** — not even `ENCLOS` at +24, which round 1 (all of us)
  treated as the one fixed offset. I keep `ENCLOS` at +24 anyway (§3)
  because robigo's own `Defn.h` macros for base natives and capi's
  existing code use it, and moving it buys nothing; but it is now a
  choice, not a constraint. `FRAME()`/`HASHTAB()` become robigo-internal
  materialization functions, invoked only by base natives compiled
  against robigo's `Defn.h` — which the brief confirms constrain nothing
  package-visible.
- **Removed:** `VECTOR_PTR`, `DATAPTR` (no longer declared),
  `Rf_isFrame`, `LOGICAL0/INTEGER0/REAL0/COMPLEX0/RAW0` declarations.
  Consequence: robigo's installed headers must *also* omit these, or
  packages that fail on GNU R 4.6.2 would pass here — a compat bug in the
  permissive direction. Header surface is generated/checked against
  4.6.2's installed headers by a diff test (§9, M2).
- **Warned (check WARNING, still declared):** `BODY`, `FORMALS`, `CLOENV`,
  `ATTRIB`, `SET_ATTRIB`, `PRCODE`, `PRENV`, `PRVALUE` (+ setters),
  `Rf_allocSExp`, `STRING_PTR`, `SET_TYPEOF`, `Rf_findVarInFrame3`, …
  Consequence: these must exist and behave exactly, but they are
  *functions over our layout*, which they already are in capi. A package
  using `PRVALUE` on a promise whose value lives unboxed in a binding
  slot never sees that promise (it sees a materialized forced promise,
  §4.2); a package using `PRVALUE` on a real PROMSXP sees PRVALUE — which
  is why the aux-word scalar variant (§0.1 dispute 3) must expand on
  *every* `PRVALUE` read, `R462/src/main/memory.c:4441`
  (`R_expand_promise_value`) being GNU R's own pattern.
- **New API to implement and to design on:** the binding/dots API
  (`R_GetBindingType` `Rinternals.h:656`, `R_MakeDelayedBinding` `:670`,
  `R_MakeForcedBinding`, `R_MakeMissingBinding`, `R_DotsElt` `:694`,
  `R_GetDotType`, `R_DelayedBindingExpression`, …; implementations in
  `R462/src/main/envir.c:879` `R_GetBindingType`, `:3801`
  `R_MakeDelayedBinding`, `:1647` `R_DotsElt`), `R_getVar`/`R_getVarEx`
  (`Rinternals.h:496-497`), `R_envSymbols` (`envir.c:3321`), `R_mapAttrib`
  (`Rinternals.h:1218`), `R_getAttributes`/`R_getAttribCount`/
  `R_getAttribNames`/`R_hasAttrib`, `R_class`, `R_nrow`/`R_ncol`,
  `R_altrep_class_name`/`R_altrep_class_package`, `DATAPTR_RW`,
  `R_getRegisteredNamespace`. The binding API is the important one: GNU R
  has now blessed *exactly* the abstraction G3 needs — a binding has a
  type (value/missing/delayed/forced/active) independent of storage.
  `robigo-rt`'s internal `BindingKind` (§3.2) is that enum, so capi's
  implementation of the new API is a near-identity mapping instead of a
  simulation, and rlang-style consumers get the fast path GNU R designed
  for them.
- **Not freed:** observable R-level behaviour of `ls` order, `inspect`'s
  `tl=`/REF counts, `tracemem`, environment printing, serialization
  formats (`.rda` compat is observable), and the whole *documented* C API
  (`REAL`, `INTEGER`, `VECTOR_ELT`, `PROTECT`, ALTREP registration, …).
  4.6 frees storage, not semantics.

#### 0.5 Best idea adopted from each rival, and one rejected

- **Astra — adopted: a value is not kept alive by its parent's lifetime.**
  Astra's §1 observation that borrowing a child off a rooted parent is
  unsound if the parent's edge can be replaced is the sharpest soundness
  point in round 1. In my design it becomes: child `Ref<'g>`s are sound
  *only because* no edge can change during `'g` (edge writes need `&mut
  Cx` or a store through `Unique`, both of which end `'g`), and allocation
  auto-roots (§1.2) so a collection during `'g` traces the parent edge.
  Also adopted: two-phase builtin kernels (gather/materialize with roots,
  then a callback-free slice kernel, §1.4) and transactional one-way
  exposure of frames to C (§3.3).
  **Rejected: scope-branded `Value<'s>` as the only safe handle, with a
  root registered on every creation including every child read.** It pays
  a root store per `car()` in pairlist walks — the hottest shape in the
  evaluator — to defend against parent-edge replacement that the
  `&mut`-ends-`'g` rule already makes impossible. The VM then bypasses it
  with raw slots anyway (Astra concedes this), so the strong guarantee
  would cover exactly the code that needs it least.
- **Fable — adopted: variables live in the environment object in inline
  typed slots; there is no variable register cache and no flush.** This
  deletes `Vars`/`flush_pending`, makes `get()`, `sys.function` and C's
  `findVar` see current values by construction, and is the single biggest
  simplification available. Also adopted: the 16-byte typed value-stack
  slot as the universal precise root container (§2.2), the ZST `Unwind` +
  isolate `Pending` making `RResult<Value>` one word (§4.5), static
  immortal objects (§5), and `ROBIGO_NO_INPLACE` (B8).
  **Rejected: the two-pass mark (precise decides finalizers, conservative
  words pin afterwards), for Rust and for C.** The synthesis names the
  failure: an EXTPTR reached only by a pin is finalized — its C finalizer
  runs and frees the resource — while a Rust local or C local still holds
  the pointer and will use the resource. Fable's defence ("cannot corrupt
  memory") is about the robigo heap block, not the resource the block
  points at. GNU R's failure mode for the same missing-PROTECT bug frees
  block and resource together; Fable's frees the resource and keeps the
  block valid, which converts a crash into silent use-after-free of
  package state. Experiment A is still worth running (§10) as a
  falsification test for C pin-only, but the burden of proof is now on
  showing no package in the suite holds an unprotected EXTPTR across an
  allocation — a negative I do not expect to be provable *(guess)*.
- **Opus — adopted: `Ref<'g>` branding with the `&Cx`/`&mut Cx` split, and
  plan B migration.** Also adopted: weak inline-cache words cleared by the
  collector (his §7.3 R6-finalizer argument is correct and I had missed
  it: a strong call-site cache holding a method closure keeps its
  environment — the R6 object — alive past `gc()`), `Out` slots as the
  return convention for R-running code, and the source-scanner ban on
  branded types in heap containers.
  **Rejected: deferred warnings for "pure" builtins, and with it the
  conservative scan of the innermost Rust segment that his design needs.**
  The two are one decision: `&Cx` code cannot signal (warning handlers run
  R code), so warnings queue; and unrooted `Ref`s across `&Cx` allocation
  live only because the scan sees them. I keep the branding and refuse
  both prices: warnings take `&mut Cx` at the C call site (§4.5), and
  allocation auto-roots (§1.2). His `ROBIGO_SOLE` process-global (his §8,
  open question 3) I also reject: it reintroduces exactly the
  process-global mutable state the isolate record exists to remove, for
  one well-predicted branch's worth of gain. One TLS read at C entry,
  passed down, is the design (§8).

---

### The full design

New and changed files (details per section):

| File | Change |
|---|---|
| `crates/robigo-rt/` | new crate: the runtime. `src/obj/` (branded API), `src/eval.rs` (reference interpreter), `src/compile.rs` + `src/vm.rs` (one IR), `src/jit/`, `src/env.rs`, `src/builtins/` (re-ported), `src/isolate.rs`, `src/roots.rs`, `src/context.rs` |
| `crates/robigo-core/src/{lib,gc,heap}.rs` | per-type node sizes (B3a), ENVSXP/PROMSXP typed-slot tracing, value-stack + context root ranges, weak cache words, symbol arena, static immortals, collector variant behind feature for D9 |
| `crates/robigo-core/src/stack.rs` | new: the typed value stack (§2.2) |
| `crates/robigo-capi/` | shrinks to exports/trampolines/headers over `robigo-rt`; `bridge.rs`, `host.rs` deleted at flip |
| `crates/robigo-runtime/` | deleted at flip (B2) |
| `crates/robigo-text/` | extracted as planned (`docs/architecture.md` "Text"); both runtimes use it during migration, so it lands first and survives |
| `scripts/gen-names/` | new: builtin registry generator (B6) |
| `tests/r/passing-rt/`, `tests/corpora/*/passing-rt/` | new ratchet column (§9) |
| `docs/ledger/g3.md` | new: decision table results as they land |

#### 1. The Rust-side object API (council item 24)

##### 1.1 Layers and types

Three layers, as Opus's table but with rooting made precise throughout:

```rust
// crates/robigo-rt/src/obj/mod.rs
pub struct Cx { /* §8: heap + all isolate state; !Send + !Sync */ }

/// A live object, valid for the shared borrow 'g of the isolate.
/// Invariant: every Ref<'g> is reachable from a precise root, a heap
/// edge of a reachable object, or the current alloc frame (§1.2).
###[derive(Clone, Copy, PartialEq, Eq)]
###[repr(transparent)]
pub struct Ref<'g> { p: Sexp, _g: PhantomData<&'g Cx> }

###[derive(Clone, Copy, PartialEq, Eq, Hash)]
###[repr(transparent)]
pub struct Sym(Sexp);            // immortal; unbranded; dense id (§5)

pub struct RootScope<'s> { /* base index into Cx.roots; !Copy */ }
###[derive(Clone, Copy)] pub struct Root<'s> { slot: u32, _s: PhantomData<&'s ()> }
pub struct Out<'s> { slot: u32, _s: PhantomData<&'s mut ()> }   // a value-stack slot (§2.2)
pub struct Args<'a> { /* window into the value stack + tags; §4.3 */ }
pub struct Global { idx: u32 }  // Cx global root table (envs, caches of objects)
```

`Ref<'g>::view(self) -> View<'g>` is the only match point. `View` variants
mirror today's `Value` enum (`crates/robigo-runtime/src/value.rs:885`)
under the same names where they exist (`Null, Sym, Char, Lgl, Int, Real,
Cplx, Raw, Str, List, Expr, Pairlist, Lang, Dots, Closure, Builtin,
Special, Env, Promise, ExtPtr, WeakRef, Bytecode, S4`), each payload a
`#[repr(transparent)] Copy` typed newtype. Reads (`car/cdr/tag`, `len`,
`attrib`, `formals/body/cloenv`, `prvalue/prcode/prenv`) are
`#[inline(always)]` loads at core offsets (`crates/robigo-core/src/lib.rs`
layout block: type +0, flags +1, gp +2, refcnt +4, extra +6, attrib +8;
vectors length +16, truelength +24, payload +32; nodes car +16, cdr +24,
tag +32, aux +40) and return `Ref<'g>` of the same brand. Soundness of a
child `Ref` rests on the §0.5 Astra condition, stated as the module's
contract: **during `'g`, no heap edge changes**, because every edge write
goes through `Sexp::store` wrappers that take `&mut Cx` or a `Unique`
(§1.4), and C cannot run. Allocation during `'g` may collect, but the
parent is reachable (invariant above) so the child is traced.

##### 1.2 Allocation auto-roots: the alloc frame

The one place branding alone is silent: a fresh allocation's result is
reachable from nothing. Opus kept it alive with the stack scan; Astra
with an explicit scope root per creation. My mechanism, cheaper than
Astra's and precise unlike Opus's:

- `Cx` holds `alloc_frame_base: u32` into the root stack (§2.3). Every
  R-entry and every builtin dispatch (§2.3) records the root-stack top;
  that recording *is* the alloc frame.
- Every `&Cx` allocation (`cx.alloc_real(n)`, `cx.cons(..)`,
  `cx.mk_char(..)`, …) pushes its result onto the root stack before
  returning the `Ref<'g>`. Cost: one store + one increment per
  allocation, identical to GNU R's `PROTECT` cost that capi's bench
  already measures at 1.00× GNU R (Fable §2.3, citing the core ledger).
- Taking `&mut Cx` (any R/C-running call, any warning) first truncates
  the alloc frame to the recorded base — which is exactly when `'g` ends
  anyway. A value that must survive is promoted explicitly: stored into a
  heap edge, an `Out`, or a `RootScope` slot, all of which the compiler
  forces by type (the `Ref` cannot be named after the `&mut` call).
- Builders (§1.4) allocate once and fill; the single auto-root covers the
  whole fill.
- The VM does not use alloc frames per op: its results land in
  value-stack slots, which are already precise roots (§2.2). Alloc frames
  serve builtins and the reference interpreter.

Prototype decision D1 (§10) measures this against Opus's scan variant and
Fable's explicit `keep()`; if auto-root costs >2% on `calls.R` against
explicit `keep`, the API falls back to explicit `keep` + branding (the
brand still catches cross-callback errors; only cross-allocation errors
revert to torture-tested discipline). The fallback is decided now.

##### 1.3 The heap-container ban

A `Vec<Ref<'g>>` in Rust heap memory is invisible to every root
mechanism. Three plugs, from Opus §1.2, all from day 1:

1. `cx.ref_vec()` returns a `RefVec<'g>` backed by the root stack
   (contiguous root slots above a recorded base; no inline small-buffer
   — root slots *are* the buffer, so the small-buffer optimization buys
   nothing).
2. `crates/robigo-rt/tests/no_heap_refs.rs` scans the crate's sources and
   fails on `Vec<Ref<`, `SmallVec<…Ref`, `HashMap<…Ref`, `Box<…Ref`,
   `Rc<…Ref`, `Option<Box<…Ref` outside `obj/` — the
   `tests/no_inner_jumps.rs` pattern.
3. `ROBIGO_GC_STRICT=1` in CI (§2.4) with poisoned free slots.

Long-lived Rust-side object references (caches, `Code` metadata) use
`Global` (strong, traced) or weak cache words (§7.3) — never a `Ref`
field. `Ref` appearing in a struct definition outside `obj/` fails the
same scanner; views are stack values by construction.

##### 1.4 Borrowed reads, `Unique`, builders

Typed read API per vector kind (shown for `Real`):

```rust
impl<'g> Real<'g> {
    pub fn len(self) -> usize;                    // ALTREP-aware: Length only, never forces
    pub fn std(self) -> Option<&'g [f64]>;        // inline payload, no code run
    pub fn elt(self, i: usize) -> f64;            // own ALTREP classes only (&Cx-safe)
    pub fn regions(self, buf: &mut [f64; 512]) -> Regions<'g, '_, f64>; // own classes
}
impl Cx {
    pub fn dataptr_ro<T: Elt>(&mut self, x: Root, out: /* … */) -> RResult<()>; // may run package code
    pub fn read_regions<T: Elt>(&mut self, x: Root, f: impl FnMut(&[T]) -> RResult<()>) -> RResult<()>;
}
```

The safepoint rule is the borrow checker: a `&'g [T]` slice borrows
`&'g Cx` (through the typed handle), and every safepoint — allocation is
*not* one for slice validity (non-moving heap, marks in side bitmaps;
the object is rooted) but R/C evaluation, warnings, ALTREP package
methods, `SETLENGTH`-growth and in-place writes are — takes `&mut Cx`.
Two-phase kernels (Astra): phase 1 with `&mut Cx` validates, coerces,
materializes and roots; phase 2 downgrades to `&Cx` and runs a plain
slice kernel. Code that must interleave (coercion warnings mid-vector)
drops the slice, signals, and re-acquires via `Real::std` again,
re-reading length and data pointer; `obj` provides
`cx.revalidate(x: Root) -> Real<'_>` for exactly this. Under
`debug_assertions`, views carry a `Cx` safepoint epoch and `Deref`
asserts it — catches safepoints smuggled through `&Cx` interior
mutability (my r1 mechanism, kept; cost zero in release).

Mutation goes through one type:

```rust
pub struct Unique<'g, T>(T, PhantomData<&'g ()>);   // !Copy; only writable handle
impl Cx {
    pub fn writable<T: VecKind>(&mut self, x: Root) -> RResult</* writes Unique into Out */>;
    pub fn alloc_real(&self, n: usize) -> FreshVec<'_, f64>;  // builder (§1.2 auto-rooted)
}
impl<'g> FreshVec<'g, f64> {
    pub fn data(&mut self) -> &mut [MaybeUninit<f64>];
    pub fn set_elt(&mut self, i: usize, v: Ref<'g>);          // VECSXP/STRSXP fill: incref-only
    pub fn finish(self) -> Ref<'g>;
}
```

`writable` is GNU R's `if (MAYBE_SHARED(x)) x = shallow_duplicate(x)` at
the places the C has it, reporting through `tracemem`'s hook. It takes
`&mut Cx` not because duplication runs R code (it does not) but because
it must end every live slice of *any* object (the duplicate decision for
`x` must not coexist with a slice of `x` obtained through an alias);
`unique_mut2(x, y, |xs, ys| …)` (Opus) covers read-while-write kernels
with an `x != y` check. Builders are the fresh-object fast path: core
prefills STRSXP/VECSXP with nil/blank so a fresh vector is GC-safe at
every instant; `set_elt` on an unpublished object skips barrier and
old-value decref (bulk incref only). Without builders, filling a
million-element VECSXP through `Sexp::store` (`lib.rs:605`: barrier
test, two refcount updates) is a self-inflicted regression against
today's `Vec` push — my r1 core change B, kept.

`unsafe` is confined to `robigo-rt/src/obj/`, `robigo-core`,
`robigo-capi`, and the evaluator core (`eval.rs`, `vm.rs`, `jit/`) which
works on raw `Sexp` + value-stack slots under core.md §3 rules.
Everything else in `robigo-rt` is `#![deny(unsafe_code)]` per-module.

##### 1.5 Copy-on-modify: REFCNT + deferred link counts, exactly GNU R's

- `REFCNT` (u16 at +4, saturating) counts heap edges and bindings only,
  maintained by `Sexp::store`. `MAYBE_SHARED` (>1), `MAYBE_REFERENCED`
  (>0) under GNU R's names. The sweeper never decrements a dead parent's
  contribution; counts overstate, never understate; that is observable
  through `.Internal(inspect)` and `tracemem` and is therefore spec, not
  sloppiness. `R_CleanupEnvir` and `unpromiseArgs` (R450 `eval.c:2132,
  2169`) are ported exactly — they are what lowers argument counts at
  return.
- **Deferred link counts for stack values** (the r1 gap): port
  `R462/src/main/eval.c:997-1050`. The isolate holds
  `links_committed: *mut Slot` into the value stack. Before any in-place
  modification reached through a binding (`STARTASSIGN` points: subassign,
  replacement calls, `SetVar` of an object), `commit_links()` increments
  every object slot in `[links_committed, current_base)` and advances the
  mark — one compare when already committed. When an activation's window
  pops, `uncommit_to(base)` decrements. Within one activation, the
  compiler emits `IncLnk`/`DecLnk` for a register loaded by `GetVar` (not
  a fresh result) that is live across an assignment in the same frame —
  GNU R's `INCLNK`/`DECLNK`. The reference interpreter does the same at
  `evalList` points (R450 `eval.c:3666,3679`). `ROBIGO_NO_INPLACE=1` (B8)
  is the differential oracle: in-place modification is unobservable when
  correct, so any output difference between modes over the corpora is a
  link-protocol bug, full stop.
- Replacement builtins decide as their C does
  (`if (MAYBE_SHARED(x) || (!IS_ASSIGNMENT_CALL(call) &&
  MAYBE_REFERENCED(x)))`, R450 `subassign.c:1633`). AGENTS.md porting
  rule 5 ("take arguments out of `args` with `mem::replace`") is retired:
  arguments arrive in a rooted value-stack window (§4.3), the builtin
  writes its (possibly duplicated) target back into `args` slot 0, and
  `applydefine` stores it. Validation-before-mutation remains mandatory.

##### 1.6 What a builtin looks like

```rust
pub type BuiltinFn = for<'a> fn(cx: &mut Cx, call: Root<'a>, args: Args<'a>,
                                env: Root<'a>, out: Out<'a>) -> RResult<()>;
```

The dispatcher records root-stack and value-stack tops before the call
and restores them after (§2.3), so early `?` returns need no cleanup.
`args.get(i).bind(cx) -> Ref<'_>`; results are written to `out`.
Registration uses the generated table (B6); `prim/special/internal`
become generated attributes, not per-module hand data.

#### 2. Rooting Rust frames: precise everywhere in Rust, conservative only in C

##### 2.1 The decision and the rejected alternatives

A collection marks, in order: (1) precise roots — the protect/root stack
(§2.3), the precious set, API globals, the `Global` table, the value
stack `[0, top)` (tag-checked, §2.2), the context stack (§4.4), handler/
restart stacks, the pending-exit slot (§4.5), per-symbol isolate cells
(§5); (2) weak-reference processing and finalizer decisions on those
marks; (3) conservative scan of **C segments only**, as strong roots,
with today's narrowed trampoline boundary and register spill
(`Core::close_scan_segment`, core.md §3; the 2026-11-11 fixes);
(4) CHARSXP prune on the union; sweep. Rust frames are never scanned, at
explicit `gc()` or otherwise. Finalizer determinism at `gc()` then holds
by construction, not by segment emptiness.

Why not the rivals (full arguments in §0.5/§0.1): whole-stack or
innermost-segment conservative-as-roots reintroduces the measured
finalizer nondeterminism (core ledger 2026-10-28, 2026-11-11 — including
a stale pointer in a `Value`'s padding bytes) and is unsound in the
other direction under LLVM (register-only and negatively-derived
pointers are invisible; JSC/Ruby accept this because they control their
compilers or their value tagging — `heap/ConservativeRoots.cpp`,
`gc.c:mark_current_machine_context` — robigo controls neither rustc nor
Cranelift's register allocation). Pins-after-weak (Fable) finalizes live
EXTPTRs. The systems whose discipline I copy: V8 handle scopes
(`src/handles/handles.h`, scoped slots — here, root-stack slots + alloc
frames), Lua/CPython's "values live on the VM stack" (here, §2.2), Julia
GC frames for generated code (`src/julia.h` `JL_GC_PUSH` — here, JIT
spills to slots, §7.4), and GNU R's own `PROTECT` for C, which our C
segment scan already exceeds. SpiderMonkey needed a static hazard
analysis (`js/src/devtools/rootAnalysis`) to make precise rooting hold
in C++; branding is our hazard analysis, run by rustc on every build.

##### 2.2 The value stack

One contiguous typed stack per isolate (`crates/robigo-core/src/stack.rs`),
GNU R's `R_BCNodeStack` generalized (R450 `Defn.h:1609`; scanned by tag
in `memory.c`):

```text
Slot (16 B): w0  tag in low bits if < 4096 (1 REAL, 2 INT, 3 LGL, 0 empty),
                 else an object pointer (objects are ≥16 B aligned heap addrs)
             w1  scalar bits when tagged; unused (zero) for objects
```

Holds: VM register windows, builtin argument windows, the reference
interpreter's evaluated-argument runs, JIT spill slots. The collector
scans `w0` of every slot below `top`: ≥4096 ⇒ mark. No NaN-boxing
(NA_real_ is payload 1954 and NaN payloads are compared bit-for-bit).
Reserve 256 MiB VA, commit on demand, guard page; overflow raises GNU
R's "evaluation nested too deeply"/node-stack errors at the same points
`R_BCNodeStackTop` checks do. Dead registers are cleared to tag 0 at
last use (compiler-emitted; the #357 discipline) — prototype Q3
measures clearing vs per-safepoint liveness bitmaps. A 16 B slot against
today's `RVal` enum (24–32 B by its fields, `vm.rs:41`; exact size mine
to measure in D3) also shrinks register traffic. AoS-16 vs SoA
(vals+tags, Opus §7.1, 9 B/slot) is prototype D3; I start AoS because
the tag test and payload load share a cache line in scalar dispatch
*(guess — D3 decides)*.

##### 2.3 The root stack and alloc frames

One Rust root stack per isolate (`Cx.roots: Vec<Sexp>`, pre-grown),
**separate from `R_PPStack`**: C may `UNPROTECT(n)` across a callback
into Rust under GNU R's contract; it must never pop Rust roots. Core
receives `roots[..top]` as a root range via a dedicated
`Core::set_rust_roots(*const Sexp, *const u32)` slot registered at
isolate creation (cheaper than a `root_fns` callback per collection;
`gc.rs` root-fns mechanism exists today). Discipline:

- Builtin dispatch, R-entry (`eval`, `apply`, `force`, VM entry, native
  call wrapper, finalizer runner) and context push each record
  `(roots.len(), vstack_top, pp_top)`; normal return and `Err` unwind
  both truncate to the record. `RootScope` drop is the fast path only —
  correctness never depends on `Drop`, because a C `longjmp` into a
  trampoline skips destructors; context records (§4.4) carry the saved
  tops and `in_context` restores them, as capi's `errors::Ctx` restores
  `R_PPStackTop` today (core.md §5). `tests/no_inner_jumps.rs` extends
  to `robigo-rt` unchanged.
- The alloc frame (§1.2) is the region of the root stack above the
  innermost R-entry record that the current `&Cx` stretch has pushed.
- Dead-slot hygiene: a root slot is overwritten with nil when its owner
  knows the value is dead before scope end (large builtin loops use
  `mark()/release(m)` brackets). Precise-but-stale roots delay finalizers
  exactly as stale stack words did — Astra's point; the finalizer stress
  script at varying depths (§10) is the test.

##### 2.4 Checking the discipline

`ROBIGO_GC_STRICT=1`: gctorture (collect at every allocation past a
threshold) + poisoned freed slots + root-stack canaries. CI runs
conformance + corpora slices + capi corpus under it on the `rt` column
(core/capi already run under torture, core.md §3). Release builds keep
a counter of root-stack high-water and alloc-frame pushes in
`ROBIGO_GC_STATS` — health metrics, not safety nets. There is no pin
scan in release to forgive a missing root; that is deliberate. A missing
root is a CI failure under strict mode, never a production heisenbug.

##### 2.5 JIT frames

Native code holds objects only in value-stack slots or environment
slots; machine registers hold scalars and data pointers derived from
slot-rooted objects (non-moving heap ⇒ derived pointers stay valid
across allocating helpers). No stack maps, no statepoints, for Cranelift
now and any LLVM tier later (§7.4).

#### 3. Environments

##### 3.1 Layout: variable-size ENVSXP, inline typed slots

```text
ENVSXP, 48 + 16n bytes (n = slot count; n = 0 for born-general envs)
+0   header   gp: FRAME_LOCK (1<<14), GLOBAL_FRAME (1<<15) as GNU R;
              extra: RDEBUG | ESCAPED | SEARCH | BASE (robigo bits)
+8   attrib
+16  frame    NULL while pure-slot; materialized FRAME pairlist once exposed (§3.3)
+24  enclos   ENCLOS — kept at +24 by choice (§0.4), OFF_ENCLOS
+32  index    NULL, or RAWSXP open-addressing sym→slot index (n > 8 or hashed envs)
+40  aux      slot frames: shape (STRSXP-like VECSXP of Sym, owned by Code)
              general envs: version box (§3.4) or NULL
+48  slots[n] (§3.2)
+48+16n  order[n]: u16 creation ranks (§3.5)   [in the same allocation]
```

Frame words +16/+32/+40 are ours (and under 4.6, +24 is too, §0.4).
`Core::alloc_env(shape, enclos)` allocates header+slots+order in one
size-class carve; a 3-formal call frame is 96 B + ranks in **one**
allocation, against today's 240 B `EnvData` plus binding buffer (Fable's
count from `env/trace.rs:78`, `env.rs:60`). Growth beyond `n` (a binding
the compiler did not see: `assign("z",…)`, `eval` in frame, C
`defineVar`) promotes the frame to *extended*: overflow bindings go into GNU
R-shaped binding cells on the `frame` pairlist, slots stay authoritative
for their symbols, lookup checks slots then (only if `frame != NULL`)
the pairlist. One extra NULL test on the miss path; the hit path is
unchanged. Removal (`rm`) of a slot sets `R_UnboundValue` and keeps the
slot (indices stable ⇒ cached slot hints stay valid); compaction never
happens for slot frames (they die with the call); for long-lived general
envs, removal follows GNU R cell semantics.

##### 3.2 Slots and binding kinds

```text
slot (16 B):  key  Sym address | bit0 LOCKED | bits1-2 MISSING (GNU R's 0/1/2)
                        | bits3-5 kind
              val  object Sexp, or scalar bits when kind says so
kind: 0 object · 1 REAL · 2 INT · 3 LGL · 4 active (val = function)
      5 eager-arg object · 6 eager-arg REAL · 7 eager-arg INT   (§4.2)
```

Symbols are 64 B-aligned in their arena (§5), so 6 low bits are free —
GNU R's typed binding cells do the same via `extra`
(`R462/src/include/Defn.h:464`, with the `BNDCELL` macros at `:483-506`).
The tracer's new ENVSXP case marks `val` for kinds 0/4/5 only (and
`frame`, `enclos`, `index`, `aux`, attrib as today). `BindingKind` in
Rust is 4.6's `R_BindingType_t` (`Rinternals.h:647-654`: Unbound, Value,
Missing, Delayed, Forced, Active) with kinds 1-3/6-7 reported as Value
after expansion and kind 5 as Forced. Reading a slot as an object boxes
an unboxed scalar **into the slot** (kind → 0/5) — GNU R's
`R_expand_binding_value` (`R462/src/main/memory.c:4396`), one function
serving `get`, `as.list(env)`, C `findVar`, and promise-of-variable.
That store-back is what fixes #276 (C writes to purrr's loop index: C
writes the box the slot now holds; the VM re-reads the slot, never a
flushed register copy — there are no variable registers, §0 item 5).

Slot writes: scalar → two stores (key constant-folded, bits), with a
decref if the old kind held an object; object → `Sexp::store` (barrier
+ FIX_REFCNT). The VM addresses own-frame locals as `env + 48 + 16k`
after a per-frame `shape == code.shape` check that folds away (the VM
made the frame). Lookup by name (reference interpreter, `get`, C):
shape scan ≤8, else the `index` table (open addressing on `Sym` id,
tombstones on remove).

##### 3.3 Exposure to C and to GNU R-shaped views (one-way, transactional)

Under 4.6 no package can name `FRAME`/`HASHTAB`, but warned entry points
and base natives still can reach frame structure (`BODY/FORMALS/CLOENV`
analogues for envs: `R_findVarLocInFrame`, `FRAME()` in robigo's
`Defn.h`). Rule, from Astra: exposure materializes **canonical GNU R
binding cells once**, in a callback-free commit while env and cells are
rooted; thereafter the cell CAR is authoritative and slot reads for that
symbol redirect through the cell (slot kind → object, val = cell's value
kept in sync by making the *cell* the store and the slot a pointer to
it… precisely: on exposure of symbol *s*, allocate the cell, move the
value, and mark the slot `kind = 0, val = cell`; slot read/write helpers
special-case `val` being a binding cell exactly as GNU R's
`BNDCELL` accessors do). Never a copied snapshot with write-back — that
is the twin failure mode (#189, #217) and it is banned by construction.
A retained varloc stays valid because cells never move and slot indices
never change. `R_GetBindingType` & friends (§0.4) read slot kind
directly and do **not** trigger exposure — that is what the 4.6 API is
for, and the common rlang path therefore never materializes cells.

Direct spine mutation by base-native C (`SETCAR` on a FRAME cell) is
visible because the cell is the store. Spine *structural* mutation
(prepending a cell) bumps the env's version (§3.4) via the `frame`
word changing; cached lookups that assumed "not in frame" recheck
`frame != NULL` cheaply.

##### 3.4 Versions, guards, global cache

- Slot frames need no version for the common guard ("does lookup of
  `sym` from here still resolve to base?"): if `sym ∉ shape` and
  `frame == NULL` (not extended), the frame cannot bind `sym` — a
  compile-time fact plus one load. This deletes the fresh-frame guard
  miss the perf-vm ledger prices at ~5,000 Ir per `calls.R` iteration
  (Fable §3.4, citing the 2026-11-10 entry).
- General envs get a **version box** on demand: a 16 B RAWSXP in `aux`
  holding `fn_version: u32, bind_version: u32`, allocated the first time
  a guard wants to memoize against that env. Memos hold the box (strong
  `Global` or a `Code` consts entry — a traced edge) plus the seen
  version. **No cache anywhere in `robigo-rt` keys on a bare address**:
  key on an object the cache keeps alive, or on a never-repeating
  counter (Fable's rule, adopted as a lint-reviewed invariant; address
  reuse already bit tracemem once).
- Locked namespaces with locked bindings: guards are static (checked
  once at namespace seal).
- Global cache: GNU R's `R_GlobalCache` (R450 `envir.c:616-772`) stored
  per symbol in the isolate's symbol-cell table (§5): `(env, cell-or-
  slot, epoch)`. `search_epoch` (in `Cx`, bumped on attach/detach/import
  changes) validates wholesale; per-symbol flush on `GLOBAL_FRAME`
  mutation as GNU R. Replaces both `Interp::fun_cache` and VM search
  memos for the global tail of a lookup. Lookup of a function from the
  global env: one indexed load + one epoch compare.
- Free-variable inline caches per `GetVar` site: `{hops, shape₁…shapeₕ,
  slot}` — each hop checks shape identity (a traced `Code` const, so no
  address-reuse hazard) and not-extended. `<<-` uses the same cache then
  a slot store with `commit_links()` first (§1.5) — the `bal <<- bal + x`
  row of `calls.R`, 1.39× GNU R today *(per brief/round-1 ledgers)*,
  becomes ~8 loads *(estimate, D2 measures)*.

##### 3.5 Order, locking, the base env

Enumeration order is observable and must match GNU R byte-for-byte:
unhashed frames list later definitions first then matched arguments;
hashed frames list by GNU R bucket order. `order[n]` ranks record
first-definition order; listing a slot frame = sort by (segment, rank)
with the formals/post-formals transform today's `frame_names`
(`env.rs:284-291`) already implements. For hashed general envs, listing
computes GNU R's `R_Newhashpjw` bucket order over the live symbols at
list time (cold path; the table itself is ours, B5). Active bindings:
kind 4; reads/writes call the function (`&mut Cx`), no version can
cache through them. Locking: env lock = gp `FRAME_LOCK`; binding lock =
slot bit 0 / cell gp; error texts verbatim from `envir.c`. Base env and
base namespace share one binding store the way GNU R shares `SYMVALUE`:
both ENVSXPs are general envs whose slot block *is* the per-isolate
symbol-cell table (§5) — two env objects, one store, heap-visible to C
identically; today's `FrameCell::Shared` (`env.rs:406-411`) becomes a
layout fact instead of an `Rc` trick.

#### 4. Closures, promises, argument matching, contexts, unwind

##### 4.1 Closures and code

```text
CLOSXP (48 B): +16 FORMALS · +24 BODY (always the source LANGSXP — body(),
               deparse and warned BODY()/FORMALS()/CLOENV() need no unwrap)
               +32 CLOENV · +40 aux: Code or NULL
Code (private BCODESXP-shaped, 64 B): +16 consts VECSXP (constants, shapes,
               cache target objects — traced) · +24 source expr
               +32 shape · +40 weak-cache block (§7.3)
               +48 *mut CodeData (ops, JIT state; sweeper release fn) · +56 spare
LANGSXP (48 B): +16 CAR · +24 CDR · +32 TAG · +40 aux: Code for this
               expression (eval'd language objects cache their compilation)
LISTSXP (40 B under B3a): +16 CAR · +24 CDR · +32 TAG, no aux
```

The `function` special copies `Code` from the `function` call's aux into
the new closure (closures in a loop share code; code dies with the last
closure — deleting today's never-evicting `CACHE` thread-local,
`vm.rs:219` per Fable/Opus). `SET_BODY`-equivalents from warned C entry
points clear aux. Native code in a dead `CodeData` is released by the
sweeper outside collection, and only retired once no JIT frame can be
inside it (isolate retire list drained at top level — the bridge's rule
for C-replaced bodies, kept). GNU R BCODESXP from serialization keeps a
`Code` whose `CodeData` holds today's decompile-to-expression payload.

##### 4.2 Promises and eager arguments

```text
PROMSXP (48 B): +16 PRVALUE (R_UnboundValue unforced) · +24 PRCODE (always)
                +32 PRENV (NULL once forced) · +40 aux: Code of the thunk
                gp 0-1 PRSEEN · variant D4: gp tag + aux = scalar bits
```

- **Eager arguments** (Fable §4.2, generalizing today's caller-forced
  promises and `BIND_MISSING_DEFAULT`): where the strictness analysis
  proves the caller may evaluate an argument (today's `Code::strict` /
  `caller_promargs` conditions, perf-vm ledger — same guards, same
  rejection memos), the value is written into the callee's slot with
  kind 5/6/7 and **no PROMSXP is allocated**. `missing()` reads slot
  bits 1-2; `substitute()` takes the expression from the call (GNU R's
  `promiseArgs` uses `CAR(el)` of the call as PRCODE, so identity
  matches); forcing returns the value. When an object is demanded —
  C `findVarInFrame`, `R_ForcedBindingExpression`, frame escaping its
  call, `as.list(env)` — the slot materializes a forced PROMSXP
  `{PRVALUE = value (boxed if scalar), PRCODE = call arg expr, PRENV =
  NULL}` and kind becomes 0. This closes #277 (C-built vs runtime-built
  promise divergence: there is one kind of binding, and what C sees is
  forced iff R has read it — also fixing the constant-default gap
  core.md §6 records) and prices at zero promises for the fib/`f(i*0.5)`
  shapes. `ROBIGO_MATERIALIZE_TORTURE=1` (B8) materializes at bind time;
  output must not change.
- Constant defaults: bound as kind 5/6/7 with MISSING bits marking
  "missing with default", exactly today's mask semantics moved into the
  slot. Non-constant defaults: a PROMSXP in the slot (kind 0) whose
  PRENV is the callee frame.
- Forcing a PROMSXP: PRSEEN 1 during, 2 = interrupted (the "restarting
  interrupted promise evaluation" warning, restored by context unwind
  as R450 `context.c:170`); PRVALUE write-back through `store`; PRENV →
  NULL. The VM's `force` returns into an `Out` slot unboxed where the
  value is scalar, with the box written to PRVALUE at the same commit
  point (no deferred write-back window in which C could see an unforced
  promise R has consumed — the r1 designs all had some flavour of this
  hazard; B8's torture mode exists to catch regressions).
- **D4 variant** (Opus): after forcing, a scalar PRVALUE may be stored
  as bits in aux under a gp tag, tracer skipping aux for that tag,
  expanded by `R_expand_promise_value`-equivalent on every PRVALUE read
  (GNU R's own `IMMEDIATE_PROMISE_VALUES`, `R462/src/include/Defn.h:38`).
  Adopted only on D4's numbers (§10); the binding-slot design does not
  depend on it.

##### 4.3 Argument matching and `...`

Arguments are evaluated left-to-right into a value-stack window
(`Args`: base slot, n, `*const Sym` tags from the call site — no
`Vec<Arg>` allocation, deleting `Interp::arg_pool` traffic). Matching
ports `match.c`'s three passes over the window, with the two fast paths
in front (positional-direct, exact-tag via shape index — today's
`bind_positional`/`bind_exact`, `interp.rs:2263,2288`). Per call site, a
matching **plan** (arg i → slot j / dots / dropped-unused-with-warning,
missing formals) is cached in `CodeData`, valid while
`callee.FORMALS` is the identical object (held in `consts`) and the
callee is the identical closure (weak word, §7.3). `...` binds as a
DOTSXP pairlist of PROMSXPs — GNU R's shape, because rlang walks it with
CAR/CDR and 4.6's `R_DotsElt`/`R_GetDotType` read it — built at bind
time only when the callee has `...` in formals (always, if present;
dots forwarding splices without copying promises). The 4.6 dots API
reads: Delayed = unforced PROMSXP; Forced = PRVALUE set (or a dots
element that arrived eager — represented in the pairlist as a forced
PROMSXP, since dots elements *are* objects the moment the pairlist
exists). `..n`, `...length()`, `...elt()` are VM ops over the cons list.
Frame exit ports `R_CleanupEnvir` (clear promises/dots/bindings, ENCLOS
→ empty env when unreferenced, discounting self-cycles); under the D9
recycling variant, an unreferenced unescaped frame's memory returns to
a LIFO free list instead.

##### 4.4 Contexts

One `#[repr(C)]` context stack in the isolate
(`crates/robigo-rt/src/context.rs`), replacing `Interp::frames`
(`interp.rs:51-77`), capi's `CONTEXTS` and `errors::Ctx`. Field set from
GNU R's `RCNTXT` (R450 `Defn.h:1319`), robigo's layout (96 B):

```rust
###[repr(C)]
pub struct Ctx {
    callflag: i32, evaldepth: i32,
    call: Sexp, callfun: Sexp, cloenv: Sexp, sysparent: Sexp,
    promargs: Sexp,            // NULL until demanded; built from (args window, plan)
    conexit: Sexp, srcref: Sexp,
    handler_top: u32, restart_top: u32,
    pp_top: u32, roots_top: u32, vstack_top: u32,   // §2.3 restore triple
    vmax: u32, flags: u16, _pad: u16,               // intsusp, gcenabled, browserfinish…
}
```

Records live in a pre-grown `Vec` whose addresses are never published to
C (C sees robigo `Defn.h`'s RCNTXT only through `begincontext`/
`endcontext` exports pushing/popping this stack, plus field accessors —
base natives are compiled against our header). `Sexp` fields are
precise roots (core root range over `ctx[..top]` field offsets, or a
`root_fns` walk — offsets generated from the struct by a build check).
`promargs` materializes lazily for `sys.call`/`match.call`/`sys.function`
/C. Handler and restart stacks are isolate `Vec`s of `#[repr(C)]`
entries with `Sexp` fields (precise roots the same way); GNU R's
pairlist form is materialized only if a census entry point reads it
(check `tests/capi/symbols.tsv` at M2; *(guess)* none does).
`context.c` ports onto this: `begincontext`, `endcontext`,
`R_run_onexits`, `R_sysframe/call/function`, `R_restore_globals`
(including PRSEEN=2 restoration and pending-promise handling).

##### 4.5 `Flow`, warnings, and C ↔ Rust ↔ C

```rust
pub struct Unwind;                       // ZST: RResult<Ref<'_>> is one word where used
pub type RResult<T> = Result<T, Unwind>;
// in Cx, a precise root:
pending: Pending { kind: ExitKind, target: u32 /* ctx index */, value: Sexp,
                   condition: Sexp, call: Sexp }
```

(Fable §4.5 / Opus §4.5 converged here; I take Fable's ZST form.)
`Flow::error`/`error_call` keep their names and AGENTS.md's call-choice
rules (`errorcall` → the builtin's own call; `error` → context call).
**Warnings**: `cx.warning(call, msg)` / `cx.warningcall(..)` take
`&mut Cx` and signal *immediately* at the ported C call site — handlers,
`warn=2` escalation, muffle restarts all run there, because warning
handlers are R code and R code is `&mut`. There is no deferred queue and
no purity list. The one concession to ergonomics: pure slice kernels in
phase 2 (§1.4) return a small `Diag` enum (which warning, at which
index) to their phase-1 caller *within the same builtin*, which signals
before producing any observable effect and before returning — that is
not deferral across an R-visible boundary, it is ordinary control flow
inside one ported C function, and the C source's own order (warn at
first bad element vs warn after loop) is preserved by construction
because the porter places the signal where `warningcall` sits in C.
Boundary rules are core.md §5 unchanged: Rust never `longjmp`s; only
`extern "C"` wrappers convert `Err`→jump; C entries that may jump go
through the `robigo_try_*` trampolines; `R_UnwindProtect` continuations
are isolate-owned; on-exit code runs during unwind with `pending`
rooted. C→Rust→C→Rust nesting composes because each boundary converts
exactly once (Astra's A/B/C/D walk holds verbatim).

#### 5. Strings and symbols

`RString`/`RStr` are deleted. A string element is `Char<'g>` (CHARSXP,
8 B in a STRSXP vs 16 today); `NA_character_` is the immortal
`NA_STRING`, compared by address — never `None`. The CHARSXP cache,
encoding marks (ASCII/native/UTF-8/Latin-1/bytes in gp) and weak
interning are core's existing machinery (`docs/core.md` §4,
`crates/robigo-core/src/chars.rs`), called through `obj/`:

- `c.bytes() -> &'g [u8]`, `c.enc()`, `c.is_na()`, `c.hash() -> u32`
  (cached at +8).
- `c.as_str() -> Option<&'g str>`: ASCII bit ⇒ the bytes; UTF-8 mark
  alone does not prove validity in GNU R, so validity is computed once
  and cached in two spare `extra` bits (Fable).
- `cx.translate_utf8(c) / translate_native(c) -> Cow<'g, [u8]>` at
  exactly the points the ported C calls `translateCharUTF8`/
  `translateChar` — translation is a call-site property, never a read
  property. Shared text machinery lives in `robigo-text` over `&[u8]`.
- CHARSXP +12 (spare u32 beside the +8 hash) caches `sym_id + 1` (0 =
  no symbol): `install(CHARSXP)` becomes two loads. Hot path of
  `get("x")`, `e[["x"]]`, computed `attr()`, `as.name` (Fable).
- `TRUELENGTH` (+24) stays package scratch exactly as today (hidden in
  4.6 headers, but the *field* is observable to already-compiled usage
  patterns via other routes and costs nothing to preserve).

**Symbols.** `Sym` is the SYMSXP address (r1's `u32` deleted, §0 item 7).
SYMSXPs are 64 B, 64 B-aligned, immortal, in a dedicated arena:
`+16 PRINTNAME · +24 spare · +32 INTERNAL · +40 aux (nil) · +48 id u32 ·
+52 hash u32 · +56 spare`. `Sym::id() = (addr - arena_base) / 64` —
dense, no load. Alignment is what frees the 6 tag bits in binding-slot
keys (§3.2) and in value-stack `w0`. Per-isolate symbol data lives in
`Cx.syms: Vec<SymCell>` indexed by id, 16 B each: `base_value: Sexp`
(GNU R's SYMVALUE — the base env's store, §3.5) and
`global_cache: (env, loc, epoch)` (§3.4). Symbols shared across
isolates later need only interning synchronization; mutable cells are
already per-isolate (core.md §10 direction).

**Static immortals** (Fable §1.6): `R_NilValue`, `NA_STRING`,
`R_BlankString`, `TRUE/FALSE/NA` logicals, `R_UnboundValue`,
`R_MissingArg`, and the predefined symbols become static objects in the
binary, so `v == NULL` and tag compares are link-time addresses with no
load. Core changes: `mark_obj` already returns early on IMMORTAL; the
store barrier's remembered-set path (which finds the set via the block
header) gains a static-object branch. Anything with mutable attributed
children is *not* static (core.md:310 warns the current "immortals" have
mutable children) — the audit list is per-object, in M2.

**Performance obligations** (prototype Q7, §10): `paste` builds in a
reused `Vec<u8>` and interns once per result — interning always, because
CHARSXP pointer equality is load-bearing for packages (data.table) —
but the intern-always cost against today's uninterned `Rc<str>` is
measured, not assumed. `match`/`unique`/`duplicated`/`%in%` port
`unique.c`'s string paths: pointer hash + pointer compare when both
sides are cached with compatible encodings, UTF-8 translation only
otherwise. Sorting/collation unchanged (in `robigo-text`).

#### 6. Vectors and ALTREP

Storage is core's header-flag taxonomy, now the only taxonomy:

| Variant | Representation | Access rule |
|---|---|---|
| Owned | inline payload +32 | slices under §1.4 |
| Compact seq | robigo ALTREP intseq/realseq | `elt`/`regions` inline the arithmetic; materializes only on writable/`DATAPTR` demand — `1:2e8` constant memory (#460, a G3 exit gate) |
| Package ALTREP | class methods via trampolines | `&mut Cx` accessors call methods exactly where GNU R calls them (`length(x)` → `Length` only; no eager copy, no deferred-wrapper whitelist — both deleted) |
| Foreign (Arrow/mmap) | ALTREP class | RO via `Dataptr_or_null`; RW materializes once; `DATAPTR_RW` (new in 4.6) implemented for class authors |
| EXTDATA | foreign writable buffer + release fn | survives only for genuinely C-owned writable memory; every bridge use (`HOST_OWNED` twins) deleted |

Private SEXPTYPE space: GNU R uses 0–25, 30, 31 (Opus, from
`Rinternals.h`); robigo takes 27 = `Code` (§4.1) and leaves 26/28/29
reserved, unallocated. Under B3a the size-class table is per-type; type
bytes are never visible to C for 26/27 (no accessor reaches them).

**Growth.** R-visible numbers are GNU R's: `GROWABLE` gp bit,
`TRUELENGTH` = capacity, `EnlargeVector`'s 1.05 policy (R450
`subassign.c:140,177`) — `.Internal(inspect)` prints `tl=`. Invisible
optimization, adopted from Fable/Opus convergence: when GNU R's new
capacity still fits the allocation's size class (or a large chunk's
reserved tail VA), extend in place and set length/truelength to GNU
R's values without copying; R cannot observe the difference except via
`.Internal(address())`, which is already address-space-dependent.
`x[n+1] <- v` in a loop is then amortized, not GNU R's repeated 5%
copies — faster *and* indistinguishable. Builders (§1.4) cover the
unpublished-construction case. Attributes: one canonical pairlist at
+8; `attrib.c` ported once in `robigo-rt`, capi wraps it (deleting
capi's parallel 773-line `attrib.rs`); 4.6's `R_mapAttrib`/
`R_getAttrib*` are thin views over it. The S4 bit lives in the header
from day 1 (unblocks G6; `IS_S4_OBJECT` hidden in 4.6 but `IS_S4_OBJECT`
semantics via `Rf_isS4` remain API).

#### 7. Compiler, VM and JIT on Sexp

##### 7.1 One IR (B4)

`robigo-rt/src/compile.rs` lowers LANGSXP → a register IR over
value-stack slots (§2.2), carrying: ops with slot operands, per-op
liveness (for slot clearing and `IncLnk` placement, §1.5), strictness
analysis results, shape/slot resolutions, call-site records, and the
frame `shape` (formals + `findLocals` locals). The VM executes the IR;
Cranelift tiers consume the same IR + liveness. The reference
interpreter (`eval.rs`) is a naive `eval.c` port over LANGSXP used for
bootstrap bring-up (before the compiler exists, M3 order in §9) and
permanently as the differential oracle (B4/B8). It is never the default
tier for compiled closures; uncompiled top-level code compiles through
the same IR at first execution, as today's VM does.

##### 7.2 What carries over from perf-vm — and what is redone

Carries (the ledger's "For G3" list, `docs/ledger/perf-vm.md:2048ff` in
r1 citations): call-site function lookup (now shape tests + global
cache, §3.4); pure-closure and nested pure-call inlining (slot in/out);
caller-forced promises (now eager slots, §4.2); index promises forced
in R's order; guard memoization (shapes + version boxes); static
rejection memos; `%*%` row-vector dot products; unboxed returns (the
return `Out` slot); whole-closure native call trees and loop summaries
(retargeted); the measurement method (`BYFN` compare, asm diffing).
Redone, not ported: `SmallVec` register/slot buffers (→ value stack);
`set_var`/`own_var` `Rc::get_mut` tricks (→ slot stores + REFCNT);
`Vars`/flush (→ deleted, §3.2); `run_with` cold-path layout (re-measured
from scratch on the new dispatch; keep the hot loop small from commit
one); `RVal::Range` (→ compact ALTREP object, one 48 B allocation per
loop — or a range *descriptor in the w1 word* of a tagged slot if D3
shows the allocation matters *(variant, measured)*).

Registers are value-stack slots; variables are environment slots. A
VM-to-VM call: push window, `alloc_env` (or recycle, D9), bind by plan,
push context, run — allocations per general call: 1 (frame) + 1 per
lazy argument + 1 per dots element, against GNU R's frame + cons-per-
binding + promise-per-arg + promargs pairlist.

##### 7.3 Inline caches are weak

Every cache that would hold a closure, environment or class vector
stores a **weak word**: the `Code`'s cache block (+40) is registered
with the collector, which zeroes words whose targets are unmarked
after marking, before sweep (the CHARSXP prune mechanism, generalized).
Strong retention by a cache is a conformance bug (Opus's R6 argument:
`obj$method()` in a call-site cache keeps `obj` alive past `gc()`).
Cache *keys* that must stay strong (FORMALS identity for a matching
plan, shape objects) live in `Code.consts` as traced edges — those are
code objects, kept alive by the closure that owns the code, which is
the correct lifetime. S3 dispatch gains the cache Fable proposed:
keyed on the identity of the class STRSXP (weak word) + the method
binding cell, validated by cell value — dispatch costs ~18.8k Ir today
(perf-vm ledger, r1 citations) and key identity is exact while the
weak word survives. New cache the layout makes cheap: verified
free-variable slot hints `{hops, shapes, slot}` (§3.4), the ledger's
named next step.

##### 7.4 JIT ABI

```rust
type LoopFn = unsafe extern "C" fn(cx: *mut Cx, slots: *mut Slot, base: u32) -> u32;
```

Native code reads/writes slots directly (`slots + 16*slot`, tag in w0)
— no `VecView` marshalling (today's `jit.rs:121-134` deleted), no
`LoopCtx`. Object access inlines header offsets **taken from
`robigo_core` constants** (`OFF_LEN`, `OFF_DATA`, …) so a layout change
is a compile error in the JIT, not corruption: type test + flags test
(`ALTREP|EXTDATA` → helper), length load, `x+32` data. Pointer-vector
stores inline the barrier test + `FIX_REFCNT` u16 update, else helper.
At every helper that can run R code, live objects are already in slots
(deopt state = slot state, so deopt remains "write scalars back, VM
executes the op"); allocation-only helpers need no spill beyond the
slots because registers hold only scalars/derived pointers (§2.5).
In-place vector kernels check `refcnt ≤ 1` after `commit_links()` at
entry; native code runs no R code, so the check holds for the run.
Tiers: VM → Cranelift loops → Cranelift whole-closure trees, as today
in structure; an LLVM optimizing tier is *not* in G3 scope — the IR is
designed so it can be added without a second semantics (that is B4's
point), and §11 R8 prices the risk that Cranelift closure trees lag.

#### 8. The isolate record

```rust
// crates/robigo-rt/src/isolate.rs
###[repr(C)]
pub struct Cx {
    // hot prefix: fixed offsets for VM/JIT (asserted against core constants)
    vstack_top: *mut Slot, vstack_limit: *mut Slot, links_committed: *mut Slot,
    roots_top: u32, pp_top: i32, ctx_top: u32,
    visible: bool, interrupts_pending: bool, gc_inhibit: u32, eval_depth: i32,
    core: robigo_core::Core,            // heap inline: alloc fast path is 2 loads off cx
    roots: Vec<Sexp>, globals: Vec<Sexp>,
    ctx: Vec<Ctx>, handlers: Vec<Handler>, restarts: Vec<Restart>,
    pending: Pending,
    syms: Vec<SymCell>,                  // §5
    global: Global, base: Global, base_ns: Global, empty: Global, registry: Global,
    search_epoch: u64,
    frame_free: [Sexp; 8],               // D9 recycling variant
    // …every one of today's 69 thread_local!s, inventoried in M2:
    options, rng, numerics_state, connections, dlls, warnings, locale, tz,
    jit: JitState, retire: Vec<*mut CodeData>,
    capi: robigo_capi::Tables,           // opaque to rt; rt is capi's host now
}
thread_local! { static CURRENT: Cell<*mut Cx> }   // the ONLY thread_local (lint-enforced)
```

Access: Rust hot paths receive `&mut Cx`/`&Cx` (as `&mut Interp` today);
JIT code receives `cx` in a callee-saved register; C exports read
`CURRENT` once and pass it down; signal handlers only set
`interrupts_pending`. No `ROBIGO_SOLE` global (§0.5). Per-isolate C
globals (`R_GlobalEnv`, `R_PPStackTop`, …) are macros over the current
isolate in robigo's headers from the start (B7) — in a 4.6 world where
non-API variables draw check NOTEs, header macros over an accessor are
also the *more* compatible choice for documented use. Multi-isolate
concurrency is not a G3 deliverable: one R thread per isolate, `collect`
aborts off-thread as today; G3's obligation is to add no new obstacle
(everything mutable is in `Cx` or in per-isolate symbol cells; C
packages with process globals stay process-isolated).

#### 9. Migration: plan B with ratchets, re-port, flip-and-delete

##### 9.1 Shape

`crates/robigo-rt` on main from its first commit; the conformance
harness gains an `rt` column with marker dirs `tests/r/passing-rt/`,
`tests/corpora/*/passing-rt/` (ratchet rules unchanged: markers only
added, never removed). The old runtime remains the default binary until
flip. Two executables, zero shared objects between them — not a mixed
heap (Astra's framing); the mixed-heap rejection stands unanimous and I
do not reopen it: attributes/list elements/promises/envs point
anywhere, cross-heap cycles needed the bridge's one-mark machinery, and
every named identity bug (#278 #277 #276 #189 #304 #167/#389 #217
###460 #140) is a persistent cross-heap reference desynchronizing. B1
(4.6.2 retarget) lands *before* M1 on the old runtime, so neither
runtime ever has to be debugged against a moving oracle.

##### 9.2 Steps and exits

| Step | Where | What | Exit |
|---|---|---|---|
| M0 | main | B1: retarget oracle/`rlib`/expected outputs to 4.6.2 on the old runtime; `AGENTS.md` + porting contract updated (4.6.2 paths, new API list) | all ratchets green against 4.6.2 oracle; every `.Rout` change NEWS-attributable |
| M1 | main, `robigo-rt` | the prototype = rt's seed: `Cx`, `obj` (branding, alloc frames, builders), value stack, slot envs, closures/promises/matching/contexts, ref interpreter + VM subset, one C entry/callback proof; all §10 variants behind features | §10 exit criteria E1–E6 |
| M2 | main, core | per-type node sizes, typed-slot tracing, root ranges (value stack, ctx, roots), weak cache words, symbol arena, static immortals audit, collector variants for D9; header-surface diff test vs 4.6.2 installed headers; thread-local inventory → `Cx` field list | core tests + torture green; capi corpus unchanged |
| M3 | main, rt | evaluator complete: ref interpreter boots base `--vanilla`; compiler+VM; S3; conditions/restarts; `on.exit`; lazy load. Loser variants from §10 deleted before this step ends | `robigo-rt -e 1` boots base with default packages; `ROBIGO_BOOT_VERBOSE` missing-internals list is the M4 work queue |
| M4 | main, rt | builtins re-ported module by module from 4.6.2 C (§9.4), one owner per file, each PR raising `passing-rt` on its corpus slice; B6 generator lands with the first modules | per-module: rt slice ⊇ old slice; instructions ≤ old on its rows |
| M5 | main, capi+rt | capi re-pointed at rt: exports call `Cx` directly, `.Call` passes Sexps (in the rt binary the bridge is not compiled); methods.so hooks; 4.6 new API implemented (§0.4) | capi corpus + 42-package install set green under rt |
| M6 | main, rt | JIT tiers on the new ABI (§7.4) | `perf.py` rt ≤ old per benchmark, JIT rows included |
| M7 | main | **flip**: rt becomes the default binary. Same PR (or same-week PR train) deletes `crates/robigo-runtime`, `bridge.rs`, `host.rs`, core's HOST/hold/watch/twin machinery; `passing-rt` markers become `passing`; collectors/frames that lost D9/D2 deleted | every old `passing/` marker ⊆ `passing-rt/`; no benchmark >0.5% slower; package suites ≥ old (18/20 green, 99.9% blocks at base per brief); named issues closed with tests (§9.5) |
| M8 | main | libR.so re-measurement (B7); generational collector decision if D9 deferred it; `robigo-text` already landed | ledger entries with numbers |

##### 9.3 Divergence control (the plan-B risk, priced)

Main moves fast; two runtimes drift. Controls, all mechanical: (1) from
M1, old-runtime semantic fixes merge only with a committed conformance
test, which auto-enters rt's target set; (2) CI reports the rt deficit
(old-passing ∖ rt-passing) per area; after M4 starts the total deficit
is a ratchet — it may only shrink; (3) new builtins/stats ports land in
rt first from M4 (the old runtime gets them only if a package ratchet
blocks on the old binary — expected rare, each an explicit exception);
(4) syntax/numerics/text/core/capi-header work continues unimpeded —
both runtimes consume it. Pause, from M1: VM/JIT micro-tuning tied to
`Value`/`Env` layout in the old runtime, new bridge mechanisms (fix-only
for package ratchets), `RString` work, and any prep whose only purpose
was my r1 swap (`view()`-over-`Rc` narrowing — explicitly cancelled).
The old runtime is in maintenance, and saying so in `docs/workstreams.md`
at M1 is part of the step.

##### 9.4 Re-porting ~60k lines of builtins

Port from 4.6.2 C, per GNU R source file → per `builtins/*.rs` module,
one writer per file (workstreams protocol). The porting contract is
rewritten for rt (lands at M1, reviewed like code): `PROTECT` across
allocation → nothing (alloc frame) or `Root` (across `eval`);
`errorcall/warningcall` → `&mut` methods at the same site; `MAYBE_SHARED`
tests copied verbatim; `DispatchOrEval` → `cx.try_dispatch` first. A
`syn`-based codemod in `scripts/g3-codemod/` handles the mechanical
residue when transplanting an old module whose shape already follows C
(pattern renames `Value::X` → `View::X`, `.data` → `std()`/`regions`,
constructors) — the porter decides per module whether codemod-then-fix
or fresh port is faster *to verify*, and records the choice. What no
codemod decides: rooting across callbacks (compiler-flagged), in-place
decisions (diff against C), warning sites (§4.5). Each module's done-
test is its corpus slice in the rt column under `ROBIGO_GC_STRICT=1`
and `ROBIGO_NO_INPLACE=1` differential — conversion errors in rooting
or link protocol fail there, not in production.

##### 9.5 When the bridge dies, issue by issue

At M7, with the flip, never earlier (the old binary needs it until
deleted). Deletion order inside the flip train: HOST proxies + `host.rs`
(env/clo/prom are nodes), twins/watches/holds + EXTDATA argument path
(vectors are core objects), write-backs/retire lists/`FLOWS`, then
`bridge.rs`. Each open two-heap issue closes with a named mechanism and
a regression test in the rt column *before* flip: #278/#304 address
identity (one object); #277 one binding kind (§4.2); #276 slot
store-back (§3.2); #189 one ENVSXP to clone; #167/#389 no copies exist;
###217 no write-back pass; #460 compact ALTREP everywhere (§6); #140
header RTRACE bit + `writable` reporting (§1.4).

#### 10. Prototype and measurement plan

M1's prototype is `robigo-rt` itself, scoped to run
`benchmarks/suite/calls.R` **unmodified** (all six idioms: default-left-
to-promise, environment method, `<<-`, S3, `tryCatch`, `c(i,2,3)`),
plus fib and binarytrees. Every variant is a cargo feature in the same
binary; every decision row has a pre-registered rule; losers are
deleted before M3 completes. Metrics throughout: callgrind Ir per
call/iteration by N-vs-2N differencing (`scripts/perf.py` method),
allocations/bytes per call from core `GcStats`, peak RSS, pause
p50/p99/max, materialization counts. Wall time secondary; release
profile only, never `--quick`.

| # | Decision | Variants | Workload | Pre-registered rule |
|---|---|---|---|---|
| D1 | rooting across allocation | alloc-frame auto-root (§1.2) · explicit `keep()` · Opus innermost-segment scan (control) | fib, calls.R, binarytrees; finalizer stress (depths 0..20 × 0..20, 1000 ASLR runs); strict torture | scan variant must additionally show 0 missed finalizers to even tie — expected to fail (§2.1); between precise variants, auto-root wins unless >2% Ir over `keep` on calls.R |
| D2 | frame layout | inline slots 48+16n · env + separate frame block · GNU R pairlist cells | calls.R (all rows), fib, escaped-frame/closure-churn scripts | inline wins unless pairlist is within 5% Ir (then take GNU R's, simpler C story) |
| D3 | register slots | AoS 16 B · SoA 8+1 B · range-in-w1 vs range object | fib, scalar loops, `for` over `1:n` | lowest Ir+L1-miss estimate; tie → AoS (simpler GC scan) |
| D4 | promise scalar | boxed PRVALUE · aux-bits + gp tag | `f(i*0.5)`, defaults-heavy calls.R rows, `substitute`/`missing` torture | aux-bits only if ≥3% on those rows and 0 conformance diffs |
| D4b | eager args | eager slots (§4.2) · forced PROMSXP always | fib, calls.R; `MATERIALIZE_TORTURE` differential | eager unless it loses Ir (it should not — it *removes* allocations) |
| D5 | frame reuse / young GC | LIFO recycling + ESCAPED · plain mark-region | binarytrees, cyclic-closure script | part of D9 |
| D6 | link protocol cost | deferred commit (GNU R) · eager incref on slot write | `x[i] <- v` loops, `l[[i]] <- x`, `<<-` row; `NO_INPLACE` differential must be clean under both | deferred unless >2% slower (it is GNU R's; exactness of REF counts in `inspect` already forces its *semantics*, D6 only prices implementation) |
| D7 | strings | intern-always CHARSXP · intern-always + small-string caches | `paste`/`match`/`unique`/`sprintf` loops, `strings.R` | ≤ old runtime Ir, else the delta is ledgered with a named cause before M3 |
| D8 | node sizes (B3a) | universal 48 B · per-type (cons 40 B) | pairlist-heavy: `substitute`, deparse/print, binarytrees RSS | per-type if ≥2% Ir or ≥5% RSS on any, no regression >0.5% |
| D9 | collector (B3b) | mark-region · + sticky generational · + recycling (D5) | binarytrees, calls.R, one package suite (data.table) | adopt a variant only if p99 pause or total Ir improves ≥10% with RSS ≤ +5%; else plain mark-region ships G3 and generational returns as its own measured project |
| A | Fable experiment A (falsification) | pins-after-weak on today's main, C only | capi corpus + package suite + an EXTPTR-finalizer reproducer I will write for the purpose | if the reproducer finalizes a live EXTPTR (expected), pin-only is dead for C too and the result is ledgered; segments stay |

**Exit criteria for starting M3 at scale** (all must hold):
E1 every `calls.R` idiom ≤ 1.0× GNU R 4.6.2 instructions in the rt VM,
JIT off (today 1.1–1.8× vs 4.5.0 per brief); E2 fib/binarytrees ≤
today's robigo JIT-off in Ir and estimated cycles, binarytrees RSS ≤
today's; E3 strict-mode + finalizer stress clean (0 misses in 1000
runs); E4 `NO_INPLACE` and `MATERIALIZE_TORTURE` differentials clean on
all prototype tests; E5 three real builtin modules (one arith, one
string, one env) ported with zero `unsafe` outside `obj/`, each within
1.2× its old line count, ergonomics reviewed by a council member who
did not write them; E6 capi corpus `eval`/`pairlist`/`unwind`/`extptr`
cases pass against the prototype with no bridge compiled in. If E1
fails, the decision table says which variant flips; if E1 fails under
*all* variants, G3 still proceeds (one heap is worth it alone) but the
failing idiom becomes a named M6 obligation — decided here so a miss is
data, not a crisis.

#### 11. Risks and open questions, ranked

| # | Risk | What resolves it |
|---|---|---|
| R1 | **Missing precise roots in re-ported builtins** (60k lines written fresh against a discipline `Rc` made invisible). Silent UAF. | Branding (compile-time across callbacks) + alloc frames (across allocation) + strict torture per module (§9.4) + `no_heap_refs` scanner. Residual: unsafe evaluator core — bounded to `eval/vm/jit`, reviewed as core.md §3 code. Still the top risk; budget review here. |
| R2 | **Link protocol misses a case** → in-place write visible through an uncounted alias → silent wrong answers. | `ROBIGO_NO_INPLACE` differential over corpora in CI forever (B8); `tracemem`/`inspect` conformance for the over-copy direction. |
| R3 | **Boot cliff / divergence in plan B**: rt runs nothing until base boots; main drifts meanwhile. | Ref interpreter first (boots with far less than the VM); leak-count via `ROBIGO_BOOT_VERBOSE` at M3 start — if >150 missing internals, the leaf-only reverse bridge with its shrinking table (§0.2.1c); deficit ratchet (§9.3). |
| R4 | **Eager-slot / lazy-promargs materialization misses a reader** (`match.call`, `sys.function`, a warned C entry point, 4.6 dots API). | `MATERIALIZE_TORTURE` differential; `tests/r/basics/caller-forced-promises.R` in rt column; an explicit reader census against `envir.c`/`eval.c` at M3. |
| R5 | **B5 hashed-env order** diverges from GNU R in some untested corner (resize timing changes bucket order). | Order is fully observable ⇒ fully testable: targeted conformance over sizes crossing every resize threshold, hashed + `sorted=FALSE`. Pre-decided fallback: Fable's GNU R chained HASHTAB for general envs (§0.3 B5). |
| R6 | **GC pauses/RSS on package suites** (already 0.7–1.0× GNU R there; bake-off's binarytrees weak spot). | D9 with a package suite in the loop, not just microbenchmarks; flip gate includes package timing, not only `perf.py` suite rows. |
| R7 | **C raw writes defeat guards** (saved `REAL(x)`/binding-cell pointers across calls; data.table `set`). | Exposed bindings are canonical cells (§3.3) so writes land in the store; vector data pointers stay valid (non-moving, no realloc behind C); speculative guards revalidate header words after every `&mut` boundary; tests holding pointers across nested callbacks. Generational barrier (if D9 adopts) must treat DATAPTR-exposed objects as always-dirty. |
| R8 | **JIT rewrite lags** and flip waits on M6. | Flip gate is per-benchmark; the VM alone must meet E1-class bars on the general call. If closure-tree JIT lags, flip on VM + loop JIT with the tree tier as a post-flip obligation — allowed only if no benchmark regresses >0.5%, which may not hold; *(open question for the lead, priced at M6 by `perf.py`)*. |
| R9 | **4.6.2 retarget (B1) uncovers more drift than NEWS suggests** (behaviour changes not in NEWS). | B1 is deliberately *before* G3 and on the old runtime; every ratchet change must be NEWS-attributable or filed. If B1 balloons, the lead can descope to 4.5.0 + §0.4-as-forecast without touching the rest of this design. |
| R10 | **Auto-root alloc frames cost more than modelled** (root-stack traffic in allocation-heavy builtins). | D1 with the `keep()` fallback pre-decided; builders amortize (one root per vector, not per element). |
| R11 | **Static immortals** interact with fork/CoW and multi-isolate futures (mutable children of "immortal" objects). | Per-object audit at M2; anything with mutable attributed children stays heap-allocated per isolate; `mclapply` RSS measurement re-run post-flip (core.md §10). |
| R12 | **`Sym` arena address arithmetic** on wasm32 / unusual VA layouts. | wasm32 build at M2; fallback: `id` read from the +48 field (one load) behind the same `Sym::id()` — API unchanged. |

Open questions for the lead, with my answers: (1) *Ship G3 on 4.6.2 or
4.5.0?* 4.6.2, sequenced as B1 — the binding API alone (§0.4) removes
the hardest C-compat constraint on environments, and retargeting later
means doing B1 anyway with less freedom. (2) *May rt builtins diverge
from old-runtime behaviour where the old runtime is known-wrong vs GNU
R?* Yes — rt ports the C; the old behaviour gets a conformance test
showing the diff, and the old runtime is *not* fixed (§9.3 maintenance
rule). The ratchet superset at flip is over *tests*, and a test that
recorded wrong-vs-GNU output is a bug to fix in the test, by
regenerating against the oracle — never by preserving the wrong output.
(3) *Is the reference interpreter a permanent tier?* No — permanent
*artifact*, test-only execution (B4). If it is ever the fastest correct
path for some shape, that is a VM bug to file, not a tier to bless.

*End of design.*


## Round 3

### Brief

### Council round 3: review the merged G3 design

Same council, repository and read-only rules as rounds 1 and 2 (`brief.md`
in this directory). No edits to the repo, no commits, no subagents, no
builds that write into the repo.

#### What to review

`docs/g3.md` in the repository worktree
(`<repo>/docs/g3.md`,
uncommitted). It merges both council rounds (your round-2 files are in this
directory: `astra-r2.md`, `fable-r2.md`, `opus-r2.md`, `muse-r2.md`) and the
lead's decisions L1-L5 (§1 of the doc).

#### The lead's goal, verbatim

> The goal is to become the fastest R runtime while being fully
> compatible. Nothing else suffices.

and, on GNU R's C and R code:

> Eventually I'd prefer not to vendor either as I feel like we can do
> better but it could be useful as a crutch to start. And also, if they're
> just the fallback we might be able to design them away anyway.

Effort is free; no users; the end state wins (round-2 framing, still
binding).

#### Experiment A (results)

See `expA-results.md` in this directory once present (the coordinator ran
it on today's main while you review; if it is not there yet, review §15.1
as designed).

#### Deliverable

Write your review to the file named in your task message:

1. **Score out of 10**: how well the doc, as written, gets robigo to the
   lead's goal (fastest R runtime, fully compatible), and whether it is
   specific enough to start M1.
2. **Blocking findings**: anything wrong, unsound, missing or
   contradictory, each with the section, the problem, the evidence
   (`file:line` in the repo or the GNU R 4.6.2 sources at
   `.../scratchpad/r462/R-4.6.2`), and the smallest fix as replacement text.
3. **Where the doc misrepresents the council**: a position merged wrongly,
   a dropped idea that should be in, a disagreement papered over.
4. **Answers to §17's open questions.**
5. **Non-blocking improvements**, ranked.

Keep scope: review this doc; do not write a new design. Ground claims in
code; mark guesses. Final chat reply: the file path, the score and the
blocking findings in one line each.


### Astra

### Astra: council round 3

**Score: 7/10.** The merged design chooses the right broad direction: one object graph, precise Rust roots, canonical bindings, an effect-aware compiler, source-compatible C entry points, and a separately gated runtime. The lead's decisions are clear. It is specific enough to start an exploratory M1 API prototype, but not to approve its safety contract or freeze its exit criteria. The seven findings below should be resolved before M1's API becomes the basis for mass porting. They need contract changes, not a new architecture.

This is a read-only design review. No builds or experiments were run. `expA-results.md` was absent on both checks during this review; I reviewed experiment A as designed. Below, `R462/` means `<R-4.6.2 sources>/`. References to proposed behavior identify gaps in the specification, not demonstrated bugs in an implementation that does not yet exist.

#### Blocking findings

##### B1. §2: token-free generic `len` contradicts the callback rule

**Problem.** The no-token read list includes `len` (`docs/g3.md:63`), but any package ALTREP method requires `&mut Cx` (`:58`). Length is such a method. Not forcing elements is not a guarantee that the method cannot call native code, allocate, signal, or re-enter R. A generic `len()` that calls it while a borrowed slice is live defeats the safety argument.

**Evidence.** `R462/src/main/altrep.c:340` implements `ALTREP_LENGTH` by dispatching the class's `Length` method. This is a direct callback, not a header read. The doc already correctly distinguishes package region reads at `docs/g3.md:68`.

**Smallest replacement text** for the read/vector bullets:

> Token-free reads access only raw fields or robigo-owned operations proven not to call code, signal, mutate, or collect. `plain_len()` is available for standard storage and proven-pure built-in ALTREP classes. Generic `cx.len(root) -> RResult<R_xlen_t>` takes `&mut Cx` and invokes a package's Length method through the native trampoline. It does not request elements itself. Apply the same distinction to all ALTREP queries, including Dataptr_or_null. Test a Length callback that allocates and re-enters R while the caller has another rooted vector.

##### B2. §§2, 11: the safe API needs ownership and re-entry rules, not just a lifetime

**Problem.** `Ref<'g>` as a shared-borrow brand establishes a read interval. It does not identify an isolate: ordinary lifetimes can unify for two simultaneously borrowed `Cx`s. Nor do the listed root types specify how roots survive `&mut Cx`, when a root can be reset, or how callbacks acquire a capability without fabricating overlapping exclusive references to the embedded isolate. These are foundational M1 decisions. Compile-fail tests are required, but cannot replace the contract they test.

**Evidence.** `docs/g3.md:54`, `:76`, `:80`, and `:322` leave these mechanisms unspecified. The existing native layer explicitly forbids table borrows across callbacks at `crates/robigo-capi/src/state.rs:24`. My round-2 proposal specified isolate ownership, separate stable root slots, root reset rules, and a suspended capability (`astra-r2.md:72–91`, `:266`); those constraints did not survive the merge.

**Smallest insertion** after §2's type table:

> Root scopes own stable traced slots independently of the execution capability, so a live Root does not borrow `Cx` exclusively. A root can be rebound only by its owning isolate; enforce this with an invariant generative isolate brand or a checked owner identity. Scope generation and lifetime prevent use after scope release. Clearing or replacing a slot cannot leave a usable Ref to an unrooted object. Binding and publication are explicit operations, and output is rooted in the caller before callee roots are released. `Cx` is a non-Send, non-Sync execution capability over pinned interior-mutable isolate storage. Native entry suspends it and ends all field borrows; callbacks acquire a fresh capability without creating overlapping `&mut Isolate` references. M1 tests scope escape, root reset, cross-isolate use, and nested native re-entry, using Miri on the non-FFI portion. Runtime owner checks have negative runtime tests; only static brands promise cross-isolate compile failure.

Also change “the borrow checker proves rooting” to “the borrow checker bounds read intervals; the root implementation proves reachability across safepoints.” Shared-borrow allocation is a viable candidate if it truly cannot collect or signal and B6 is resolved; I am not asking to discard it merely because my round-2 default differed.

##### B3. §6: one pending exit needs a nested ownership protocol

**Problem.** An error token backed by one overwriteable pending record loses the original exit when cleanup code catches another error. Retained C unwind continuations also need their own payload. The doc names a pending exit but never specifies saving, rooting, restoring, superseding, or retaining it. “RResult<T> is one word” is also not generally true for arbitrary T; size depends on T and available niches.

**Evidence.** `docs/g3.md:210` specifies the singleton design. `R462/src/main/context.c:298–319` saves and restores `R_ReturnedValue` around on-exit execution. `context.c:977–993` stores value, jump mask, and target in a continuation before calling cleanup, then resumes that continuation. These are distinct outstanding states.

**Smallest replacement text** for the RResult bullet:

> `RResult` carries a small exit indication. Pending exits are traced, isolate-owned records with explicit nested save/restore or token ownership. Cleanup that catches a new exit cannot overwrite the suspended exit; an uncaught replacement exit supersedes it according to GNU R's context rules. Each retained unwind continuation owns its payload and target until resumed or released. Assert sizes for the concrete hot-path Result types rather than promising one word for arbitrary T. M1 includes return through on.exit with a caught nested error, and R_UnwindProtect cleanup that catches an error before resuming the original continuation.

##### B4. §§5, 6, 9: compiled code lacks a mutation-validity contract

**Problem.** Weak caches solve unwanted retention, not stale semantics. Frame versions and search epochs do not cover an in-place change to a closure body, formals spine, or class vector. A closure-owned code object can therefore remain attached to a changed source graph. The doc needs to say what invalidates code and match/dispatch plans, including mutations through C and callbacks within a native call.

**Evidence.** `docs/g3.md:180` supplies environment versions; §6 supplies code ownership and weak caches; §9 supplies IR and deopt, but none gives graph-mutation guards. `crates/robigo-capi/src/memory.rs:1503–1535` exposes CAR/CDR mutation of existing nodes and already has write notification machinery. Changing a child does not change its owner's address. `crates/robigo-runtime/src/compile.rs:623–633` explains precisely the separate retention problem that weak caches fix.

**Smallest insertion** in §9:

> Code, matching plans, and dispatch caches validate all mutable dependencies, including BODY/FORMALS spines, class contents, bindings, and numeric policy. Pointer identity alone is not a validity guard. Tracked writes invalidate dependent code before its next use; foreign effects conservatively invalidate assumptions that cannot be tracked, including before callbacks re-enter compiled code. Active executions deopt at the next uncommitted operation and never replay a warning, callback, or visible store. M1 includes native mutation of body/formal children without replacing the top pointer, and callback-time mutation while a compiled activation is suspended.

##### B5. §§10, 15, 16: the differential gates are underspecified or false for observable effects

**Problem.** “Runs both implementations of every builtin” needs isolation. Sequential runs share mutated environments, RNG, promise state, connections, warnings and external effects. Deep-copying arguments is insufficient. Separately, E5 requires identical output under forced duplication on *every* prototype test, conflicting with the explicit tracemem/inspect requirements. Eager promise materialization can likewise change physical allocation diagnostics.

**Evidence.** `docs/g3.md:297–300`, `:463–464`. `R462/src/main/random.c:100–114` reads and commits RNG state. `R462/src/main/duplicate.c:153–166` emits a trace for shallow duplication. These are direct counterexamples to an unrestricted same-process comparison and to all-output equality under forced copying.

**Smallest replacement text** for differential mode and E5:

> Differential builtin testing runs complete test scenarios in independent processes with the same inputs and recorded environment, selecting C in one and Rust in the other. Compare results and post-state, warnings/errors, visibility, callback order, RNG state and captured external effects. Same-process dual execution is restricted to proven effect-free operations. Non-replayable external inputs require fixtures or a dedicated oracle test, not an assumed rollback.
>
> NO_INPLACE compares ordinary semantic values and effects only in cases where duplication is unobservable. Trace counts, inspector fields, allocation diagnostics and deliberate native alias mutation have separate oracle expectations in production mode. MATERIALIZE compares the declared language-observable contract; any excluded physical diagnostics are listed explicitly and tested separately. Neither debug control weakens the production compatibility gate.

##### B6. §§3, 12, 15: deferred allocation has no failure path, and “if due” omits request size

**Problem.** Shared-borrow allocation cannot collect or invoke a condition handler on failure while Refs remain live. The design needs a no-callback failure result and an exclusive retry boundary that does not replay effects. `alloc_rooted` must test the *prospective request*, not just a previously set GC flag. The claim that a large allocation consequently peaks where GNU R does is not established. Finally, matching counters is not enough to establish exact collection/finalizer observations when collection points change.

**Evidence.** `docs/g3.md:91–104`, `:343–344`, and E4. Existing `crates/robigo-core/src/heapsize.rs:298–302` takes the requested size in `due(cells)`. `R462/src/main/memory.c:2709–2714` collects before allocation when available space cannot satisfy the request and checks limits afterward. Merely collecting when the prior poll bit is set is weaker. E4 bounds dead bytes, not allocation success, total transient live memory, or observable timing.

**Smallest replacement text** for §3 items 1 and 3, plus a gate:

> Non-collecting allocation returns either initialized storage or an unsignalled allocation request/failure. It never invokes R under `&Cx`. The caller releases read leases, preserves roots and intermediate state, and retries only the failed allocation through `&mut Cx`; no committed effect is replayed. `alloc_rooted(size)` checks `due(size)`, collection and configured limits before reserving storage, and reports failure through the normal exclusive condition path. M1 tests low heap limits and injected allocation failure after earlier visible effects.
>
> Separately specify and oracle-test collection observations: logical allocation accounting, explicit gc results, automatic collection reports, and finalizer delivery. Deferred collection must pass that contract as well as E4; copying counters alone is not evidence. If required exact observations cannot be retained, revise the collection policy before accepting it.

The blanket “every behaviour ... as bytes” in §0 also needs an explicit rule for inherently variable addresses and elapsed times (already partly acknowledged for addresses in §12). This is a contract clarification, not permission to silently normalize genuine semantic differences.

##### B7. §§3, 15.1, 17.2: experiment A cannot establish the offered safety guarantee

**Problem.** Pinning the SEXP and its graph after weak processing can preserve their storage while a finalizer frees an external buffer they reference. Passing the ASLR stress and package suites can show no detected regressions; it cannot prove memory safety for incorrectly protected native code. Strong conservative roots cannot distinguish a live unprotected word from a stale word either. There is no generally exact-and-safe third choice from these observations alone.

**Evidence.** `R462/src/main/memory.c:1503–1524` clears weak fields and invokes arbitrary C finalization. Retaining the key's SEXP storage does not constrain that function's resource release. The current experiment at `crates/robigo-core/src/gc.rs:1581–1587` explicitly changes marking order and claims frames' memory remains readable, but does not control external resource destruction. `docs/g3.md:423–427` makes the experiment decisive without this limitation.

**Smallest replacement text** for experiment A's decision rule:

> Experiment A measures compatibility and performance on the tested cases; it does not prove safety for unprotected C references. Precise reachability defines the exact-finalization mode for correctly protected extensions. Conservative pinning is only a best-effort storage-retention aid and may not protect finalizable external resources. A conservative strong-root mode is an explicit compatibility policy that may delay finalizers; do not claim both guarantees. Add an adversarial native test whose pin-only external-pointer finalizer frees a retained raw buffer. Removing scan narrowing or zeroing remains conditional on the tested supported policy, not merely on finalizer-count success.

This does not reverse L5: run the experiment. It limits the conclusion that can validly be drawn from it.

#### Where the merge misrepresents or loses council positions

- **Pin-only C marking was disputed on safety grounds, not just unmeasured.** Astra round 2 (§§1–3) and Muse round 2 (§2.1) explicitly raised external-resource finalization. Leaving it as a two-way experiment without that objection papers over the disagreement. B7 restores it.
- **The safe API lost essential qualifications.** My round-2 agreement with branding included isolate identity, scope ownership, callback capability suspension, and effectful generic ALTREP queries. The merge preserves the headline and compile-fail tests, but not the mechanism (B1–B2).
- **The nested-exit objection disappeared.** My round-2 §5 explicitly rejected an overwriteable singleton without nesting. The selected representation may remain compact, but needs B3's ownership protocol.
- **Effect-aware IR was compressed too far.** My round-2 design required mutation dependencies and deopt recipes that include promises, contexts, visibility and pending exits, with no effect replay. “Effect-aware” alone does not retain that proposal (B4).
- **NO_INPLACE's exclusions were dropped.** My round-2 verification plan explicitly separated tracemem, inspect and native alias mutation. E5 adopts the unqualified contrary claim (B5).
- **L2 legitimately supersedes my preference to keep critical old-runtime fixes.** That is a lead decision, not a bad merge. Likewise L4's optional C bootstrap and eventual zero-C target are binding; review should make their interfaces concrete rather than reopening the decision.

#### Answers to §17

1. **Can rlib stop being vendored?** Checked-in source files can be replaced by a pinned build input or a generated image of the exact closure graphs. That removes source files from the repository; it does not remove the semantic dependency. Bodies, formals, environments, attributes and applicable source metadata still have to match. Compiled code can implement execution while exact objects serve reflection. I see no demonstrated way to discard both the source and equivalent observable data under the stated contract. Treat “no vendored files” and “no GNU R-derived observable bodies” as different goals.

2. **A third C-root option?** Not with the information the current scanner has. An unprotected live pointer and a stale word can be identical inputs to the collector. Additional explicit resource leases or precise native roots can distinguish them, but require instrumentation or cooperation and are not a universal remedy for existing packages. Use the supported-package protection contract and state the fallback policy honestly (B7).

3. **Which builtins can be designed away?** Remove their generic dispatch overhead on guarded common paths: scalar arithmetic/comparison, simple length/type queries, standard-storage subset/replacement, and selected construction/reduction kernels. Preserve a callable semantic entry for dynamic function values, `.Primitive`, `.Internal`, dispatch and native entry. That entry may lower to the same IR or invoke the same kernel; it need not duplicate semantics. Conditions, environment operations, I/O, RNG, reflection and general coercion need effectful implementations even when some call sites specialize. Names and callable behavior cannot disappear because ordinary syntax was optimized.

4. **What is missing from §12?** Native mutation and cache invalidation; nested unwind continuations; full binding states and alias-preserving promise materialization; debug/trace stepping, visibility and source locations; ALTREP callback order; encoding/locale-dependent behavior; allocation-limit failures; and explicit treatment of profiling, addresses and time-dependent output. For environments, specify GNU R traversal/insertion/deletion order rather than simply “slot creation order.” For virtual promises, require one canonical materialization shared by every alias, including forwarded dots.

5. **Are the speed bars right?** The per-workload competitive goal is right. Distinguish an aspiration over every workload class from a finite benchmark acceptance gate. Wall time in exact semantics is the outcome; instructions explain it and remain a repository regression gate, but cannot substitute for wall time across a JVM and native runtimes. Add cold short-lived scripts and one-off eval/parse, allocation-heavy escaping closures/cycles, callback-heavy ALTREP and native calls, long-lived package sessions with memory plateaus, large mixed-encoding strings, and tail GC latency. Measure compilation and startup costs, native-code memory, and throughput under the supported concurrency model. The no-regression test should name frozen binaries, versions, hardware, repetitions, uncertainty and the current 0.5% policy; a literal strict less-than on noisy timing is not a reproducible gate.

#### Ranked non-blocking improvements

1. **Make C bootstrap a dependency slice with an ABI manifest.** List selected src/main translation units and every private service they require. Specify the registry adapter from VM `Args` to C's call/op/pairlist/env signature, visibility, roots and unwind. The existing base-native build is evidence that this technique is useful, not proof that evaluator-adjacent src/main files drop in unchanged. Bring the minimal capi service seam before M3; a fully consolidated capi can remain M5.

2. **Name the dependency direction.** `core → no runtime`, `rt → core`, `capi → rt`, with object-independent libraries below them. If the builtin crate is separate to forbid unsafe, put its safe facade in a lower crate or an acyclic interface. This prevents the no-unsafe rule from introducing a crate cycle when rt needs the builtin registry.

3. **Scope M1's verification bars to implemented slices.** E4 says any corpus/package suite while M1 implements a VM subset. State which representative slices must run before M1 exits, then expand the same gate as coverage grows. Keep full production gates at the flip and completion.

4. **Retain raw benchmark evidence and failure explanations.** E7's “explained and accepted” should name who accepts a regression, how it relates to L1/§13, and where the decision is recorded. Measure allocation bytes, root stores, materializations and p99/max pauses alongside E1/E2 to explain winners.

5. **Do not require NEWS to explain every oracle delta.** §14.2 should accept a reproduced source change with a minimized case and build manifest. NEWS need not enumerate every observable bug fix. Keep the immutable old/new records and case identity mapping.

6. **Make code ownership accurate for shared bodies.** “Code dies with its closure” is a useful default, but shared compiled bodies and GNU R's bytecode constants can have multiple semantic owners. Permit traced ownership by all legitimate owners while keeping speculative caches weak. Do not weaken the R6 finalizer regression test.

7. **Preserve the distinction between the flip and full compatibility.** M7 cites the full G3 gate, which is good. State explicitly that containing old ratchets alone cannot establish full compatibility: the brief already reports remaining conformance and corpus failures. M8's zero-C result is likewise a separate completion measure.


### Fable

### G3 round 3: review of `docs/g3.md` (Claude Fable 5.2)

Base: main `4cff0ceb` plus the uncommitted `docs/g3.md` and the two
experiment A switches. Read-only; nothing was built or run. `R462/` is
`.../scratchpad/r462/R-4.6.2/src`. `g3.md:N` is a line of the doc under
review. Guesses are marked "(guess)".

#### 1. Score

**7 / 10.**

- The architecture is the right one for the goal: one heap, safepoint-only
  collection with the brand as the rooting proof, GNU R's counts, slot
  frames, one engine, GNU R's C as a crutch with a ratchet to zero, and a
  measured prototype with bars written first. I would not change any of
  the eleven top-level choices.
- It is not yet specific enough to start M1. Ten places are wrong,
  contradictory or missing a rule that M1 code depends on (section 2).
  Six of them (B1, B2, B4, B6, B8, B9) decide data layouts that M1 builds:
  the pending exit, the slot kinds, the allocator's contract, the frame's
  order metadata, the code object, and which heap crate M1 links.
- With the replacement text below merged, I would score it 9 and start M1.

#### 2. Blocking findings

Each has the section, the problem, the evidence and the smallest fix.

##### B1. §6: one pending exit is not enough (`g3.md:210-212`)

**Problem.** "The pending exit ... lives in the isolate" is one record. An
exit in progress runs `on.exit` code, `cend` functions and
`R_UnwindProtect` cleanups, and that code can raise and catch its own
exit. The inner exit overwrites the outer one.

```r
f <- function() { on.exit(tryCatch(stop("inner"), error = function(e) NULL)); stop("outer") }
tryCatch(f(), error = function(e) conditionMessage(e))   # GNU R: "outer"
```

**Evidence.** `R462/main/context.c:119-159` (`R_run_onexits` calls
`eval(CAR(s), c->cloenv)` during the jump). GNU R keeps the outer jump's
target, mask and value in `R_jumpctxt`'s C locals and in the intermediate
context (`context.c:198-209` `jumptarget`/`jumpmask`, `:216-244`). Today's
robigo is safe only because the payload travels by value: `Flow`, and in
capi the token taken by `PENDING.replace(0)`
(`crates/robigo-capi/src/errors.rs:31, 108, 490`). A zero-sized error with
one isolate slot loses that. Astra raised it (`astra-r2.md:186`); the merge
dropped it.

**Fix (replace `g3.md:210-212`).**

> - `RResult<T>` is one word: the error type is zero-sized. The exit it
>   stands for (kind, target context, mask, value, condition, call) is the
>   top record of a **pending-exit stack** in the isolate, a precise root.
>   Whatever runs code during an exit (`R_run_onexits`, a `cend`, an
>   unwind-protect cleanup, a finalizer) first parks the record in the
>   context being left (GNU R's `jumptarget`, `jumpmask`, `returnValue`,
>   `context.c:198-244`) and restores it afterwards, unless that code
>   leaves by its own exit to a context beyond the one being run, which
>   replaces it as in GNU R. Test: the nested `on.exit`/`tryCatch` case
>   above, in every tier and across a C frame.

##### B2. §6: a caller-forced argument reported as "forced" is observable (`g3.md:191-196`)

**Problem.** The doc makes a caller-evaluated argument "a forced slot". In
GNU R that binding is *delayed* until the callee first reads it, and R 4.6
makes the difference public. It is also visible from plain R, with no C:
`.Internal(getBindingType(sym, env))`.

**Evidence.**
- `R462/main/envir.c:807-830` (`BINDING_TYPE`: unforced promise →
  `R_BindingTypeDelayed`, forced → `R_BindingTypeForced`, typed cell →
  `R_BindingTypeValue`); `R462/main/names.c:925` (`getBindingType`).
- The kind also depends on which GNU R evaluator ran the call. The AST
  path wraps every supplied argument, constants included, in a promise
  (`R462/main/eval.c:3781`, `promiseArgs`, the final `mkPROMISE` branch).
  Byte code pushes a constant as a plain value
  (`eval.c:8042-8050`, `PUSHCONSTARG`). So `f(1)` binds a delayed promise
  on a closure's first calls and a value once GNU R's JIT has compiled the
  caller.
- A delayed binding has an environment and a forced one does not
  (`R_DelayedBindingEnvironment`). rlang captures quosures from that
  state, so a wrong kind changes what `enquo()` returns (guess: I did not
  read rlang's source for 4.6).
- Astra said this (`astra-r2.md:178`: "An eager scalar does not become an
  ordinary value binding just because no PROMSXP was allocated"). My
  round-2 table (§4.2, row 1) had the same error as the doc.

**Fix (replace the first sentence of the bullet at `g3.md:191`).**

> - **Arguments without promise objects**: a slot's kind is the kind GNU R's
>   binding would have at the same moment, never the kind that is cheapest.
>   A lazy argument is a delayed slot (expression, environment). An
>   argument the caller evaluated early is still a **delayed** slot, with
>   the value attached as a precomputed result: observers
>   (`R_GetBindingType`, `.Internal(getBindingType)`, the delayed-binding
>   accessors, `substitute`, materialization to a PROMSXP) see an unforced
>   promise until the callee's first read, which makes it forced without
>   evaluating anything. A constant argument is a delayed slot under the
>   `Eval` profile (`eval.c:promiseArgs`) and a value slot under `Bc`
>   (`PUSHCONSTARG`). Test: `getBindingType` before and after the first
>   read, for a constant, a symbol and a call argument, on a closure's
>   first call and after GNU R's JIT threshold.

Add to §12: `| binding kinds (.Internal(getBindingType), R_GetBindingType, rlang) | promise-free arguments, caller-forced values | slot kind equals GNU R's at every moment; ROBIGO_MATERIALIZE=1 differential |`.

##### B3. §3 and §15.1: experiment A cannot settle the objection it was run for, and one rule it depends on is missing (`g3.md:94-99, 114-115, 421-428`)

**Problem 1: the doc treats pins against strong roots as a measurement.**
Astra and Muse rejected pins on a safety argument, not on a number: "a
finalizer can free an external resource even if the SEXP's allocation
stays pinned" (`astra-r2.md:17, 44, 113`; `muse-r2.md`, dispute 2). A
passing experiment A does not answer that. The answer is an argument, and
the doc should state it:

- GNU R decides finalizers on precise reachability
  (`R462/main/memory.c:1448` `CheckFinalizers`, `:1534` `RunFinalizers`).
  An object that only an unprotected C local holds is finalized by GNU R
  at that collection too, and its memory is freed at the next one. Muse's
  "GNU R at least frees both together" is not what `CheckFinalizers`
  does. The results file repeats it ("GNU R would free such an object
  outright", `expA-results.md`, last paragraph).
- So with pins, the finalizer decision equals GNU R's in every case, and
  the memory lives longer. Pins are never worse than GNU R. Strong roots
  are "better than GNU R for wrong packages" at the price of "different
  from GNU R for right ones" (the stale-word misses). §0 already decided
  that trade.

**Problem 2: the equality holds only if robigo's precise roots contain
GNU R's.** GNU R's API functions protect their own arguments while they
allocate ("callee-protect"). An unprotected external pointer with a
finalizer, passed straight to an allocating export, survives in GNU R
without a finalizer run. Under pins it would be finalized unless the
export roots its arguments precisely. Today's port does this where it
follows the C (`crates/robigo-capi/src/attrib.rs:221-223`), but libR's
glue also "holds fresh objects in locals"
(`crates/robigo-capi/src/bridge.rs:398-400`), which pins cover only for
memory. §3 does not state the rule.

**Problem 3: the evidence so far is thinner than it looks.**
- The conformance run used `ROBIGO_NATIVES=/nonexistent`
  (`expA-results.md` item 4), so it ran almost no package or base-native C
  frames. "No regression" there says little about a change that only
  matters under C frames.
- The capi corpus and the package suites (my round-2 pass criteria,
  `fable-r2.md:1183-1192`) are not in the results yet.
- What is shown is solid: the planted-stale-word test discriminates, and
  the stub costs 120 instructions per `.Call` (7,631 → 7,511).

**Fix (replace `g3.md:114-115`).**

> 3. C segments conservatively, **as pins**: marked after step 2, so a
>    stack word keeps memory allocated and decides no finalizer. This is
>    GNU R's own decision rule (`memory.c:CheckFinalizers` runs on precise
>    marks), so finalization is exact for every package, and memory
>    outlives GNU R's for a package that forgot a `PROTECT`. It requires
>    robigo's precise roots to contain GNU R's: **every export that is a
>    safepoint roots its `SEXP` arguments precisely for its duration**
>    (R's API functions protect their arguments). A per-DLL switch makes
>    one library's segments strong roots (§17 Q2).

**Fix (add to `g3.md:421-428`).**

> Pass: core and capi tests, the capi corpus and the package suites with
> natives built, under gctorture, with no regression; the stress script
> misses nothing in 30 runs per tier; and a new capi case passes: an
> unprotected external pointer with a C finalizer, passed to an allocating
> export under gctorture, is not finalized during the call.

##### B4. §3 item 3 and E4: the deferred-garbage bar measures the wrong quantity, and allocation has no failure rule (`g3.md:100-102, 460-462`)

**Problem 1.** E4 bounds "bytes allocated between consecutive safepoints
that the next collection frees". That misses the case that raises peak
memory. Take 1 GB of garbage made under the trigger, then one `&Cx`
allocation of a 2 GB result that stays live. GNU R collects before the
allocation and peaks at live + 2 GB. robigo peaks at live + 3 GB. E4
counts 0 bytes, because the collection frees nothing allocated in that
stretch. This is my own round-2 metric (`fable-r2.md:102`); it was wrong.

**Problem 2.** Rule 3 ("a builtin that allocates a large result first uses
`cx.alloc_rooted`") is a convention. Nothing says what "large" is, and
nothing catches a builtin that forgets.

**Problem 3.** The doc does not say what `&Cx` allocation does when memory
cannot be had: `R_MaxVSize` reached, or the OS refuses the mapping. GNU R
collects and retries before it raises "cannot allocate vector of size"
(`R462/main/memory.c:2920-2928`) or "vector memory limit of ... reached"
(`:2179`). A robigo that raises without collecting fails where GNU R
succeeds. My round 2 had a rule (`fable-r2.md:462-468`); the merge dropped
it.

**Fix (replace `g3.md:100-102` and add an item).**

> 3. `&Cx` allocation is for objects below `LARGE` (measured; start at
>    1 MiB). A larger object is allocated through
>    `cx.alloc_rooted(&mut self, ..)`, which collects first if a
>    collection is due, so `numeric(1e9)` peaks where GNU R does. A `&Cx`
>    request above `LARGE` while `GC_DUE` is set is counted
>    (`ROBIGO_GC_STATS`) and fails the build in CI.
> 5. An allocation that hits a limit (`R_MaxVSize`, `R_MaxNSize`, a
>    refused mapping) returns `Err` with the pending kind `NeedGc`. The
>    dispatcher collects and re-runs the builtin if the registry marks it
>    restartable (no effect before its first allocation); otherwise it
>    raises GNU R's error. The conformance slices run once under a small
>    `R_MAX_VSIZE`.

**Fix (replace E4, `g3.md:460-462`).**

> - E4 overshoot: bytes allocated between the moment `GC_DUE` is set and
>   the collection that serves it never exceed max(32 MiB, 10% of the
>   live heap) on any corpus, benchmark or package suite; peak RSS ≤ GNU
>   R's on the memory benchmarks.

##### B5. §8 and §10 contradict each other: a 24-byte header against unedited `src/main` (`g3.md:249-253, 290-292, 306-308`, D8)

**Problem.** D8 removes the `truelength` word "now that 4.6 hides
`TRUELENGTH`". 4.6 hides it from packages. GNU R's own `src/main`, which
§10 compiles unedited, still uses it:

| file | use |
|---|---|
| `R462/main/envir.c:206, 209` | `HASHPRI`, the fill count of every hash table |
| `R462/main/serialize.c:633-634, 1474-1505`, `saveload.c:833-834` | hash-table counts |
| `R462/main/radixsort.c:33-34` | scratch on CHARSXPs (`order`, `sort`) |
| `R462/main/duplicate.c:43-45` | `COPY_TRUELENGTH` |
| `R462/main/subassign.c:163, 264` | growable vectors |
| `R462/main/inspect.c:138-141` | `tl=` |

My round 2 checked only `library/*/src` (`fable-r2.md:845`); under my plan
every builtin was Rust, so it did not matter. Merged with C-first, it
does. The doc's census caveat (`g3.md:251-253`) covers packages only.

**Fix (add to the layout bullet at `g3.md:249`).**

> The 24-byte variant must also carry what GNU R's own C keeps in the
> word: the hash-table count (`HASHPRI`) in a private word of the table,
> the exact growable `truelength` in a trailing word of a growable
> vector, and CHARSXP scratch for `radixsort.c`. Either robigo's `Defn.h`
> maps `TRUELENGTH`/`SET_TRUELENGTH` onto those, or the six `src/main`
> files that use the word (`envir.c`, `serialize.c`, `saveload.c`,
> `radixsort.c`, `duplicate.c`, `subassign.c`) are Rust before D8 is
> decided. D8's bar includes this cost.

##### B6. §5: slot frames lack an unbound state and a definition order (`g3.md:160-167, 345`)

**Problem.** A shape "maps symbols to slot indexes", so a local has a slot
before it is assigned.
- The kind list (value, missing, missing-default, delayed, forced, active,
  locked) has no "unbound". `f <- function() { print(x); x <- 1 }` must
  find the global `x` first.
- §12 says "slot creation order reproduces unhashed order". Slot index
  order is static; GNU R's order is dynamic. `defineVar` on an unhashed
  frame prepends (`R462/main/envir.c:1998`,
  `SET_FRAME(rho, CONS(value, FRAME(rho)))`), and `FrameNames`
  (`envir.c:3091`) lists in that order: newest local first, then the
  arguments in formals order. `rm(x)` followed by `x <- 1` moves `x` to
  the front.
- `as.list(environment())`, `mget`, `eapply` and `ls(sorted = FALSE)` show
  it. `c(as.list(environment()), list(...))` is a common idiom, so this is
  not a corner.

**Fix (add after `g3.md:167`).**

> A slot that the shape reserves and nothing has defined is **unbound**:
> lookup continues in the enclosure, and `exists`, `ls` and `R_envSymbols`
> do not see it. Each frame keeps GNU R's listing order: a per-slot rank
> written by the defining store when the order is not the shape's static
> order (the first definition of a local, a definition after `rm`), in the
> slot's spare key bits or a side vector on the frame. `rm` returns the
> slot to unbound. Test against the oracle: `ls(sorted = FALSE)` and
> `as.list(environment())` after data-dependent definition order, `rm`
> and redefinition, `assign` of a name the shape lacks, `delayedAssign`,
> `list2env`.

##### B7. §4: "invisible in-place extension" is visible (`g3.md:146-148`)

**Problem.** GNU R reallocates in `EnlargeVector` unless the vector is
unshared, growable and has room (`R462/main/subassign.c:161-179`).
Extending in place where GNU R reallocates changes two observables:
- **Identity.** `.Internal(address(x))`, `lobstr::obj_addr(x)` and
  `data.table::address(x)` compare equal before and after in robigo and
  differ in GNU R. §12's own rule is "identity is exact".
- **`tracemem`.** GNU R's new vector is not traced: `EnlargeVector`
  allocates and `copyMostAttrib`s (`subassign.c:214-275`), and only
  `duplicate` carries the trace bit (`R462/main/duplicate.c:143-147`). A
  vector extended in place keeps the bit, so a later copy prints a
  `tracemem[...]` line GNU R does not print.

Astra said it (`astra-r2.md:214`: "Do not claim invisible doubling
without an identity/trace proof"). The doc took the opposite position
without the proof.

**Fix (replace the bullet at `g3.md:146-148`).**

> - Growth: `EnlargeVector` as written (`subassign.c:140`): GNU R's
>   expansion factor, and the same object exactly when GNU R keeps it
>   (unshared, growable, `truelength >= newlen`). The reallocation is made
>   cheap instead of avoided: a large unshared vector moves by remapping
>   its pages to a new address, with no copy (measured, §15). The
>   observable result (a new object, untraced) is GNU R's.

The remapping is a proposal, not a measurement (guess: `mremap` to a fresh
address costs page-table work only; RSS and instructions decide).

##### B8. §9: nothing says which calls run under which profile (`g3.md:261-268, 353`)

**Problem.** "`Eval` reproduces `eval.c` ..., `Bc` reproduces GNU R's
byte-code compiler" names two semantics. The doc does not say when a
closure moves from one to the other, and that moment is observable:

```r
f <- function(x) x + 1
f(1); f(1)                       # GNU R's JIT compiles f here
`+` <- function(a, b) "mine"
f(1)                             # GNU R: 2 (compiled code bound base's `+`)
```

- The moment is GNU R's JIT state machine: `R462/main/eval.c:1448`
  (`MIN_JIT_SCORE`), `:1569-1630` (`R_CheckJIT`, `MAYBEJIT`, `NOJIT`), the
  loop rule in `do_for`, and installed byte code.
- `print(f)` adds a `<bytecode: ...>` line from that moment
  (`crates/robigo-runtime/src/print.rs:181-184`).
- A closure being debugged runs the AST path again
  (`eval.c:2359-2368`, `RDEBUG` → `bytecodeExpr`).
- Today's runtime implements all of this
  (`crates/robigo-runtime/src/vm/rjit.rs:1-39`, `:463-500`). The doc's
  carry-over list (`g3.md:275-278`) omits it, and "a code object" per
  closure (`g3.md:187-190`) has no place for the state.

**Fix (add to the "One engine" bullet, `g3.md:261`).**

> Which profile runs is GNU R's decision, tracked per closure in the code
> object (the port of `eval.c:R_CheckJIT`, `JIT_score` and the JIT cache
> that `vm/rjit.rs` holds today): `Eval` until GNU R's JIT would compile
> the closure, `Bc` after, with the base bindings frozen at that moment;
> `Bc` from the first call for installed byte code; `Eval` again while
> the closure is debugged; top-level loops as `do_for`'s
> `R_compileAndExecute` decides. A code object holds both variants.

Add to §12: `| <bytecode> line of print(f), .Internal(bodyCode), compiler::disassemble, serialize of a compiled closure, bindings frozen at compile time | one engine, no BCODESXP | GNU R's JIT state per closure; a real BCODESXP is made by running cmp.R when one is observed |`.

##### B9. §14.1 M2 and the appendix against L2: one `robigo-core` cannot serve a frozen runtime and the new layout (`g3.md:39, 388, 499-508`)

**Problem.** L2 freezes the old runtime and the bridge. M2 changes
`robigo-core` under them before the flip: the mark order, no collection
in allocation, new trace arms, and (D8) the header and node sizes. The
old capi and bridge depend on the present contract: collection inside
allocation with a conservative scan (`docs/core.md:262-286`), HOST, holds
and twins. The appendix removes those "at the flip" but changes the rest
earlier. Every such change is either dual-mode code in the hot paths of
the heap or a behaviour change in the frozen runtime. The doc does not
say which. My round 2 forked the heap for this reason
(`fable-r2.md:74-76, 1054-1056`); the merge kept the single crate and
dropped the reason.

**Fix (add to §14.1, and reword M2).**

> `robigo-rt` links `crates/robigo-heap`, a copy of `robigo-core` taken
> when M1 starts. All changes in the appendix land there. `robigo-core`
> is frozen with the old runtime (L2) and deleted at the flip, when
> `robigo-heap` takes its name. No cfg-gated second layout exists in
> either crate.

##### B10. §10: `ROBIGO_BUILTIN_DIFF=1` cannot run "every builtin that has both" (`g3.md:299-301`). Blocks M4, not M1.

**Problem.** Running a builtin twice is sound only when it has no effect
besides its result. Many do: the RNG (`runif`, `sample`), connections and
files (`cat`, `readLines`, `unlink`), environments (`assign`, `rm`),
in-place replacement of an unshared argument (`[<-`), R code called by
dispatch or by `lapply`, a warning whose calling handler runs R code,
options, time.

**Fix (replace the bullet).**

> - **Differential mode:** the registry gives each builtin an effect
>   class. `pure`: `ROBIGO_BUILTIN_DIFF=1` runs both implementations on
>   duplicated arguments, with warnings and errors captured as records,
>   and fails on any difference in result, attributes or records. `rng`:
>   the same, with the generator state saved and restored between the
>   runs. Everything else is compared across processes: the builtin's
>   corpus slice runs once all-C and once with the Rust port, and the two
>   outputs must be identical.

#### 3. Where the doc misrepresents the council

1. **Pins against strong roots is presented as open and measurable; two
   members closed it the other way.** Astra and Muse reject marking after
   weak processing (references in B3). The doc cites neither and lets
   experiment A "decide". The disagreement should be recorded with the
   argument that answers it (B3), not folded into a measurement that
   cannot.
2. **Astra's allocation rule is missing from D1.** Astra holds that
   allocation takes `&mut Cx` and writes into an `Out` slot
   (`astra-r2.md:6, 16, 80, 91`). The doc adopts Astra's `Out` in the
   builtin signature (`g3.md:77`) but lists only Muse's auto-rooting
   allocator as the alternative (`g3.md:87-89`, D1). Either add Astra's
   variant to D1 or record why it is out (a root store per allocation,
   against a collection that safepoint-only makes impossible there).
3. **Three of Astra's corrections were dropped, and each is a blocking
   finding above:** the pending-exit stack (B1), binding kinds that do not
   follow the representation (B2), growth that must not change identity
   (B7).
4. **Three of my enforcement mechanisms were dropped, with the
   properties they enforce kept as prose:**
   - `#[derive(Trace)]` on isolate state (`fable-r2.md:390`). §11 moves 69
     thread-locals into `Cx` and §3 says "no Rust frame is ever scanned",
     so an untraced object field in `Cx` is a silent use-after-free. The
     root list (`g3.md:108-111`) is by enumeration and already misses the
     `R_alloc` stack (`R462/main/memory.c:1830`) and the builtin root
     scopes.
   - Frame escape as a type (`FrameRef` → `Cx::expose_env`,
     `fable-r2.md:748-753`). D5 keeps recycling under ESCAPED; the doc
     keeps only the after-the-fact check (quarantine).
   - Cache words as a type (`CodeData` may hold `Strong<Code>`,
     `Strong<Shape>`, `Sym` or `WeakWord`, `fable-r2.md:1282`).
   Proposed text for §2, after `g3.md:83`: "Every struct reachable from
   `Cx` derives `Trace`; a field that holds an object is a traced type or
   does not compile. A frame becomes an object reference only through
   `Cx::expose_env`, which sets ESCAPED. Code data holds objects only as
   `Strong<Code>`, `Strong<Shape>`, `Sym` or `WeakWord`."
5. **A claim of mine that the doc does not repeat, and that I withdraw so
   it is not merged later:** "a class vector that is the same object has
   the same contents" (`fable-r2.md:962-964`). Astra was right
   (`astra-r2.md:44`). C can `SET_STRING_ELT` it, and
   `class(x)[1] <- "b"` on an unshared `x` writes it in place
   (`R462/main/eval.c:3209`, `evalseq` duplicates only when shared). An S3
   site cache must compare the class elements (CHARSXP pointers), not the
   vector's address.
6. **Opus's C-first is merged faithfully, but its evidence is stretched.**
   The numbers at `g3.md:293-294` are for base-package natives (stats).
   `src/main` reaches much further into the evaluator than `library/*/src`
   does: `R_GlobalContext` 9 times in `objects.c`, promise fields 21 times
   in `bind.c`, `FRAME`/`HASHTAB` in `serialize.c`, `saveload.c` and
   `inspect.c`, `begincontext` in `connections.c` (7), `saveload.c` (6),
   `serialize.c` (3) (counts by `rg -c` over `R462/main`). The doc should
   call this an extrapolation and test it in M1 (section 5, item 1).

Not misrepresentations: L2 (freeze) overrides Muse's and my "fixes with a
test"; L3 overrides Muse's "retarget the old runtime first"; L4 overrides
Opus's "Rust only where faster". Those are the lead's to make and the doc
says so.

#### 4. Answers to §17

##### Q1. Can any of `rlib/` stop being vendored without an observable difference?

**The content cannot change. The checked-in copy can go.**

- **What is observable is each base closure's formals, body tree and
  environment, and the calls it makes.** Not its text: GNU R installs
  base without source references, so comments and layout are invisible
  and `print(lapply)` is a deparse. But the tree is visible through
  `body`, `formals`, `deparse`, `trace` and `debug` (which prints each
  statement). The calls are visible through `sys.calls`, `sys.function`,
  `parent.frame` counts, `traceback`, condition calls ("Error in
  match.arg(type) :"), `Rprof` stacks, S3 and S4 dispatch on user
  classes from inside base functions, and the order in which arguments
  are forced. A rewritten `sapply` that produced the same result would
  differ in at least one of these. So §10's constraint is right and has
  no exceptions I can find for closures.
- **So "do better" applies to speed, not content:** compile them ahead of
  time, inline the `.Internal` wrappers, and put intrinsics behind guards
  that fall back to the real closure as soon as an observer is possible.
- **`rlib/` is already only a fallback.** The runtime loads base from an
  installed `base.rdb` when `R_HOME` has one and evaluates `rlib/` only
  otherwise (`crates/robigo-runtime/src/bootstrap.rs:1-5, 25-33`). And
  `rlib/` holds only `base/R` and the profile (`rlib/README.md`); the R
  code of stats, utils, methods, tools and the rest already comes from a
  GNU R installation (`scripts/robigo-home.sh:10-14`). The real
  dependency is GNU R's installed tree, not this directory.
- **Recommendation.** Treat the GNU R 4.6.2 tarball (pinned by hash) as
  the one build input for both crutches. `robigo-cbase` compiles its C;
  robigo itself runs GNU R's own package build (`makebasedb.R`,
  `tools:::makeLazyLoading`) to produce `base.rdb` and the other
  packages' databases. Delete `rlib/` and the copy from a GNU R
  installation in `robigo-home.sh`. This gives:
  - no vendored R code in the repository, and no GNU R installation
    needed at run time;
  - the binding insertion order GNU R has: lazy-load databases are
    written from a sorted `ls`
    (`R462/library/base/makebasedb.R:83`,
    `R462/library/tools/R/makeLazyLoad.R:293`), which sourcing files in
    order does not reproduce (I did not check whether today's source
    path emulates it);
  - an end-to-end test of `serialize`, lazy loading and the install path
    on every build;
  - the installed-byte-code provenance that `vm/rjit.rs:18-27` needs,
    if the build runs `cmp.R` too.
- **What stays true after that:** robigo's binary still contains GNU R's
  R code, as every R implementation's does. That is the language's
  standard library, not a crutch.

##### Q2. Is there a third option between pins and strong roots?

**Yes: pins by default, with the policy chosen per DLL and a report that
finds the offenders.**

- Pins already give both halves of what the question asks: exact
  finalizers for every package (GNU R decides them the same way, B3) and
  memory safety for all. What pins do not give is *resource* safety for a
  package that holds a finalizable object without protecting it. GNU R
  does not give that either; such a package works there only when no
  collection falls in the window, and robigo's collections fall at
  different points.
- **Per-segment policy.** A C segment belongs to the routine that opened
  it, and the routine to a `DllInfo`. A segment whose DLL is on a quirk
  list is scanned before weak processing (strong roots); every other
  segment after it (pins). The cost on the default path is nothing. The
  price of an entry is that `gc()` under that package's frames may run a
  finalizer late, for that package only.
- **Finding entries is mechanical.** `ROBIGO_GC_PIN_REPORT=1`: at each
  collection, list every object that was ready to finalize or had its
  weak reference cleared and that a C word then pinned, with the DLL and
  routine of the frame. Run the package suites under gctorture with the
  report on. Every hit is a missing `PROTECT` in that package, the kind
  rchk reports, and can be sent upstream. The list should stay near
  empty.
- I would not look for a cleverer mark. Any rule that lets a stack word
  delay a finalizer brings the stale-word misses back, and experiment A
  shows those are real with the stub off (1 miss in 625 cells).

##### Q3. Which builtins can be designed away?

None of the roughly 700 `names.c` entries (my script matched 672; the
round-1 brief counts 761 builtins) can stop existing as a callable:
`do.call("if", ..)`, `sapply(x, "[", 1)`, `Reduce("+", ..)`,
`match.fun` and `.Internal(f(..))` from user code reach all of them. What
can be designed away is the **second implementation**. Four groups (counts
from `R462/main/names.c`, by a script; approximate):

| group | entries | end state |
|---|---|---|
| Language specials: `if`, `for`, `while`, `repeat`, `function`, `<-`, `=`, `<<-`, `{`, `(`, `&&`, `\|\|`, `quote`, `switch`, `return`, `break`, `next`, `missing`, `on.exit`, `substitute`, `UseMethod`, `forceAndCall`, `Exec`, `Tailcall`, `.Internal`, `::` | about 30 of the 40 primitive specials | **Fully designed away.** The VM op is the only implementation. The builtin object is a stub that compiles its call to that op. Never from C. |
| Kernels the VM and JIT already own: arithmetic, comparison, logic (`do_arith`, `do_relop`, `do_logic`), `do_math1` (26 entries), `do_math2`, `do_summary`, `do_cum`, `:`, `seq_len`, `seq_along`, `length`, `is.*`, `as.*` on vectors, `c`, `list`, `rep` | about 120 | **One Rust implementation shared** by the op and the builtin object. The generic part (attributes, recycling, dispatch, overflow warnings) is a function, not an intrinsic, and must be ported; but it is Rust from M3, because the op needs it. Never from C. |
| Subsetting and assignment: `[`, `[[`, `$`, `@` and their replacement forms, `subscript.c` | 10 entries, about 5,000 lines of C | **Ported as functions, first.** The VM owns only the scalar and in-place fast paths. The rest is the hottest generic code in R and is tied to the `*tmp*` and link-count protocol, which is the evaluator's. Taking these from C would put GNU R's counts and robigo's VM on two sides of a C boundary. |
| Everything else: strings, regex, dates, sorting, I/O, connections, serialization, system, options, RNG, LAPACK | about 500 `.Internal`s | **Ported as functions**, module by module. This is where the crutch earns its keep. |

A fifth thing designed away is not a builtin: the **closure call around a
`.Internal`** (cmp.R's `safeBaseInternals`). The JIT calls the builtin
directly under a guard, as `rjit.rs` already decides it.

Order for M4, by measured share of package-suite time (not by module
size): the third row, then `paste`/`sprintf`/`format`, `unique.c`,
`coerce.c`, `attrib.c`, `bind.c`, `apply.c`.

##### Q4. Is anything in §12 missing?

Yes. Rows to add (the first two are B8 and B2):

| observable | changed by | mechanism |
|---|---|---|
| `<bytecode>` line, `.Internal(bodyCode)`, `compiler::disassemble`, `serialize` of a compiled closure, bindings frozen at compile time | one engine | GNU R's JIT state per closure (B8) |
| binding kinds: `.Internal(getBindingType)`, `R_GetBindingType`, rlang | promise-free arguments | slot kind equals GNU R's at every moment (B2) |
| address relations: `x` after `x[n+1] <- v`, the same object from two reads of an unboxed slot | in-place extension, typed slots | GNU R's reallocation rule (B7); an unboxed slot read as an object is boxed once and stored back (`memory.c:4396`, `R_expand_binding_value`) |
| `debug`/`browser` stepping text, `trace`, `sys.calls`, `sys.frames`, `sys.on.exit`, `traceback`, `Rprof` stacks | inlining, frame recycling, one engine | a context per call GNU R makes one for, or deopt before it can be read; a debugged closure runs `Eval` with a step op per statement (`eval.c:2359`) |
| `ls(baseenv(), sorted = FALSE)`, `as.list(baseenv())`, order in `as.list(asNamespace(p))` | symbol arena with dense ids, how base is loaded | GNU R's symbol-table walk (`envir.c:BuiltinNames`) and lazy-load insertion order (Q1). I did not check what today's runtime does here. |
| "cannot allocate vector of size", `mem.maxVSize()`, `R_MAX_VSIZE`, R-level `gctorture()`, `Rprofmem` | safepoint-only collection | B4's limit rule; `gctorture(TRUE)` collects at every safepoint and every C allocation |
| order of finalizers within one collection and at exit (`reg.finalizer(onexit = TRUE)`) | weak-reference list in the new heap | GNU R's list order (`memory.c:RunFinalizers`, `R_RunExitFinalizers`) |
| `env.profile()`, `.Internal(inspect(env))` `tl=` of a hash table | no `truelength` word | `HASHPRI` kept exactly (B5) |

One row to correct: "`.Internal(address())`, `identical()` on environments
| addresses differ from GNU R already; identity is exact". Identity is not
exact while §4 extends in place (B7).

##### Q5. Are the §13 bars the right definition of "fastest"?

The shape is right: every runtime, every benchmark, both modes, with a
C floor where one exists. Five workload classes are missing, and one bar
is undefined.

1. **Cold and run-once code.** Scripts, test files, Shiny handlers and
   knitr chunks run most code once, under the `Eval` profile. D11
   measures it; §13 has no bar. Add: `Rscript -e 1`, `library(p)` for the
   named packages, and a run-once `eval` loop, each ≤ GNU R's wall time
   and instructions. "Startup ≤ today" (`g3.md:377`) compares robigo with
   itself; the goal compares it with the others.
2. **Large live heap.** robigo's collector traces the whole heap at every
   collection; GNU R's is generational. A program with gigabytes of live
   data and steady allocation can lose badly to GNU R on GC time alone,
   and no benchmark in the suite has that shape (guess: I did not read
   every entry of `benchmarks.toml`). Add: 4 GB live plus churn; bar: GC
   time and total time ≤ GNU R's. This is what should decide M8's
   "generational collection".
3. **The C boundary.** The package suites run at 0.7-1.0x of GNU R's
   time, and the boundary is a likely cause (guess: no ledger entry I
   read attributes it). Experiment A measured 7,511 instructions for a
   no-op 3-argument `.Call`. Add per-crossing bars
   against GNU R: `.Call`, `.External`, `Rf_eval` from C, `R_tryEval`,
   `PROTECT`/`UNPROTECT`, `Rf_allocVector`, an ALTREP `Elt`. Each ≤ GNU
   R's instructions.
4. **Object systems.** S4 generic dispatch, reference classes and R6
   method calls. `calls.R` has S3 and one environment method only.
5. **I/O, regex and serialization.** `read.csv`, `readLines`, `gsub`,
   `regmatches`, `sprintf`, `format`, `saveRDS`/`readRDS`, `parse`,
   `deparse`. "String building" (`g3.md:372`) covers one of these.
6. **Undefined: what "fastest" means within noise.** Say the rule: the
   metric per mode (instructions for one process, wall time for warmed
   peak), the number of runs, and the margin under which two runtimes
   tie. Without it "fastest on 72 of 76" cannot be reproduced.

Also: E1 asks for `calls.R` wall time "below Ř's and FastR's" with the JIT
off (`g3.md:453-454`). Say "one process". A VM without a JIT will not beat
their warmed peak, and M1 should not fail for that.

#### 5. Non-blocking improvements, ranked

1. **Test the C-first assumption in M1, and name the files.** §10 says
   "the `src/main` files that the evaluator does not own" and never lists
   them. Add the list (owned, never compiled: `eval.c`, `envir.c`,
   `context.c`, `errors.c`, `memory.c`, `main.c`, `names.c`, `objects.c`,
   `match.c`, `dotcode.c`, `Rdynload.c`, `altrep.c`, `gram.c`, plus B5's
   six under the 24-byte header). Add an M1 task: compile `arithmetic.c`,
   `paste.c`, `unique.c`, `coerce.c`, `bind.c` and `serialize.c` against
   robigo's `Defn.h` and count the unresolved internals. See section 3,
   item 6.
2. **E6's three pilot modules do not exercise the hard part of the API.**
   Arithmetic, `paste`/`nchar` and `assign`/`get`/`exists` are all "one
   pure section". Add one builtin that calls R in a loop (`lapply` or
   `vapply`) and one that warns inside its loop and continues (`gsub` on
   invalid input, or `as.integer` on strings is not enough because it
   warns once at the end). Those are where re-borrowing after `&mut Cx`
   is either pleasant or not.
3. **Say what a C builtin costs per call.** A `do_*` function takes its
   arguments as a pairlist, so every call from the VM conses one, and an
   internal generic gets promise objects (`bind.c` reads `PRVALUE` 21
   times). That is GNU R's cost and fine for a crutch, but it means no
   performance gate can be judged on a path that still goes through C.
   State it in §10 and make the flip's "faster on every benchmark" imply
   "no C builtin on any benchmark's hot path".
4. **§3 item 4 names a finalizer site GNU R does not have.** "After a
   native call returns" is not one of `R_RunPendingFinalizers`'s callers
   (`R462/main/eval.c:1096, 6247`; `memory.c:3093, 3101`). Drop it, or
   mark it as robigo's and say why.
5. **Make the root list complete or generated.** Add the `R_alloc` stack
   (`R462/main/memory.c:1830`), the finalizer queue and the builtin root
   scopes to `g3.md:108-111`; better, derive it (section 3, item 4).
6. **`ROBIGO_GC_MOVE=1` needs its pin rule spelled out.** "Objects ever
   handed to C are pinned" must include every object an accessor returns
   (`VECTOR_ELT`, `CAR`, `ATTRIB`, `STRING_ELT`), because C may
   legitimately keep a child of a preserved object in a static. So the
   mode pins in every export that returns a `SEXP`, and conservative C
   words pin as well. Say that the mode changes printed addresses, so
   cases that print one are excluded.
7. **Where does `Eval`-profile code live, and what invalidates it?** For
   a closure it is the code object. For `eval(e)` of a language object the
   doc is silent. `for (i in 1:n) { e[[2]] <- i; eval(e) }` mutates an
   unshared call in place, including inner nodes, so a cache keyed on the
   outer cell goes stale. Either no cache for `eval` (then D11's run-once
   cost is the whole cost) or a stated invalidation rule.
8. **`tl=` is not "the growable capacity".** GNU R's `truelength` is
   `(R_xlen_t)(newlen * 1.05)` at the last growth
   (`subassign.c:193-200, 264`), and it decides when the next
   reallocation happens. Store that number, not the slot's size class.
9. **`ROBIGO_NO_INPLACE=1` needs an exclusion list.** `tracemem`,
   `.Internal(inspect)` counts and address prints observe copies, so E5's
   "identical output on every prototype test" cannot include them
   (`astra-r2.md:318`).
10. **§2: errors raised in a pure section.** The rule list at
    `g3.md:58-62` puts "signal a condition" behind `&mut Cx`. A helper
    that holds only `&Cx` and slices must still be able to fail. Say how:
    it returns an error value that the first `&mut Cx` frame signals, with
    the context stack unchanged in between so handler order is GNU R's
    (`fable-r2.md:364-369`).
11. **§14.2: add the count.** "Cases whose expected output changed between
    versions are listed" is the first number the whole plan produces.
    Make it an M0 exit item with the count in the ledger.
12. **§0 cites PR #620 and "memory note `robigo-rivals`"** (`g3.md:26,
    365`). A design doc should cite the ledger or the script, not a
    reviewer's private note.


### Opus

### Council round 3: review of `docs/g3.md` (Claude Opus 5.5)

Base: worktree at `4cff0ceb` plus the uncommitted `docs/g3.md` and the
experiment A switches (`crates/robigo-core/src/gc.rs`, `shim.c`). I ran
nothing that writes into the repo. `R462/` is
`.../scratchpad/r462/R-4.6.2`. I mark guesses *(guess)*. I computed the
builtin counts below from GNU R's sources by joining `names.c`'s
`{"name", do_fn` rows with the files that define each `do_fn` (728 of 768
rows matched; the 40 unmatched rows use multi-line declarations). Treat
them as ±5%.

#### 1. Score: 7/10

The doc merges the two rounds well and is short enough to hold in your
head. The core of the end state is right and specific: one heap,
`Ref<'g>` branding with collection only at safepoints, GNU R's REFCNT
protocol with deferred link counts, promise-free argument slots, and one
engine. Experiment A has now confirmed the C-frame mechanism
(`expA-results.md`: pin-only plus the plain stub gives 0 misses and saves
1.6% per `.Call`).

It loses three points for three reasons:

1. **L4 (GNU R's C as a crutch) was added on top of a design that assumes
   robigo owns every layout.** The crutch files read `FRAME`,
   `TRUELENGTH`, `cptr->promargs` and `cend` directly (B2, B3). The doc
   changes all four (slot frames, a 24-byte header, lazy `promargs`, "core
   §5 unchanged") and never reconciles the two.
2. **The milestone order and the crate graph contradict L2.** M2 changes
   a core that the frozen runtime shares, and M3's crutch needs C API
   exports that M5 only delivers (B1).
3. **Two observables the current runtime already handles were dropped:**
   GNU R's JIT decisions (`<bytecode>`, BCODESXP serialization) (B5), and
   dynamic bindings and their order in slot frames (B4).

**Can M1 start?** Not quite. With B1, B4 and B9 fixed (all are text
changes), yes.

#### 2. Blocking findings

##### B1. The crate graph and the M-order contradict L2 (§1 L2, §14.1 M2/M3/M5, appendix)

**Problem.** `robigo-capi` depends on both `robigo-core` and
`robigo-runtime` (`crates/robigo-capi/Cargo.toml:23,25`). It *is* the
bridge's home. The appendix's core changes (PROMISE_TAG tracing, slot-frame
ENVSXP, a 24-byte header, per-type nodes, the symbol arena) land in M2 on
`robigo-core`, which the frozen runtime and the bridge still link.
Either the frozen runtime breaks, or someone ports the bridge to every
core change. That is real feature work, not the "CI upkeep only" that L2
allows.

Separately, M3 boots base "with GNU R's C builtins". Those builtins call
`Rf_allocVector`, `Rf_protect`, `Rf_error`, `R_GlobalContext`, ... on
`robigo-rt`'s `Cx`. That is the C API over `robigo-rt`, which the plan
schedules for M5.

**Fix (replacement for the M2 and M5 rows, plus one sentence under §14.1):**

> `robigo-rt` has its own heap and API crates from M1: `robigo-heap` (a
> fork of `robigo-core` taken at M1; the appendix's changes land only
> there) and `robigo-rtapi` (the C API exports over `robigo-rt`, no host
> table). The old runtime keeps `robigo-core` and `robigo-capi` unchanged
> until the flip deletes them. M2: heap changes in `robigo-heap` with
> their own tests and gctorture. M3 includes every export the compiled
> GNU R `src/main` files and the base natives call. M5: package loading
> over `robigo-rtapi` (`dyn.load`, `R_init_*`, registered routines,
> methods.so hooks), with the capi corpus and package suites green in `rt`.

(Fable proposed the heap fork in round 2, `fable-r2.md` §B pick 1(b).)

##### B2. The C crutch and robigo's layouts are not reconciled (§10, §5, §6, §8 D8)

**Problem 1: "no boot cliff" is overstated.** About 176 of the ~728
`names.c` entries are defined in files the evaluator owns, so they cannot
come from the crutch at all:

| file | entries |
|---|---|
| `eval.c` | 64 |
| `envir.c` | 55 |
| `errors.c` | 17 |
| `context.c` | 13 |
| `debug.c` | 11 |
| `memory.c` | 9 |
| `objects.c` | 7 |

`envir.c` alone holds `assign`, `get`, `exists`, `ls`, `mget`,
`env2list`, `eapply`, `missing`, the 4.6 binding and dots `.Internal`s,
the namespace registry, `lockBinding`, `makeActiveBinding` and others.
These must be Rust at M3. About a quarter of the builtin table is a cliff,
smaller than before but still a cliff.

**Problem 2: the crutch files read the representations G3 changes.** All
paths are in `R462/src/main`:

- **`FRAME()` on arbitrary environments:**
  - `serialize.c:1127` and `:1959` (`WriteItem(FRAME(s))`,
    `SET_FRAME(s, ReadItem(..))`);
  - `saveload.c` (2 uses);
  - `inspect.c:281`.

  Slot frames (closure frames, and every unhashed environment if D2u
  wins) have no FRAME.
- **`TRUELENGTH`:**
  - `subassign.c:163` and `:264` (growable vectors);
  - `radixsort.c:33-34` (CHARSXP scratch for `order()`);
  - `serialize.c:633-634` and `:1474-1505`, `saveload.c:833-834`
    (VECSXP hash-table counts);
  - `duplicate.c:43-45` (`COPY_TRUELENGTH`).

  D8's 24-byte header removes the word they write.
- **`cptr->promargs`:** read as a struct field in `util.c:594`
  (`do_nargs`, a hot primitive). A field that C reads directly cannot be
  "materialized lazily" (§6).

**Fix (append to §10 "Start"):**

> The crutch compiles only files the evaluator does not own. `eval.c`,
> `envir.c`, `errors.c`, `context.c`, `debug.c`, `memory.c`, `objects.c`,
> `match.c` and `main.c` are Rust from M3 (about 176 `names.c` entries;
> the registry lists them). robigo's `Defn.h` defines `FRAME`, `HASHTAB`,
> `TRUELENGTH`/`SET_TRUELENGTH` and the `RCNTXT` fields with semantics on
> robigo's layouts:
> - `FRAME` of a slot frame builds a GNU R-ordered pairlist snapshot
>   (read-only; `SET_FRAME` on a slot frame is an error);
> - `TRUELENGTH` has storage under every D8 variant for growable vectors
>   (capacity), CHARSXPs (scratch) and VECSXPs used as hash tables;
> - `promargs` is filled when a closure context is pushed, or every
>   compiled file that reads it is on the Rust list (today only `util.c`,
>   `do_nargs`).
>
> `scripts/check-cbase.sh` greps the compiled sources for `FRAME(`,
> `HASHTAB(`, `TRUELENGTH`, `->promargs`, `->cloenv` and `->callfun`, and
> fails on any use not in its allowlist.

Add to D8: "variants must keep a TRUELENGTH word for growable vectors,
CHARSXPs and hash-table VECSXPs while any crutch file that uses it is
compiled in". §7's "symbol cached in a spare word of its CHARSXP" must not
share the word that `radixsort.c`'s `savetl` overwrites.

##### B3. Unwinding through C contexts: "core.md §5 unchanged" is wrong (§6)

**Problem.** core.md §5 keeps its own jmp_buf stack in `shim.c` and lets
the trampoline catch the jump. GNU R's own C pushes `RCNTXT`s with `cend`
cleanups all over the crutch files:

- `builtin.c:629` (`cat`);
- `print.c:842`;
- `deparse.c:415` and `:501`;
- `coerce.c:1692` (`parse`);
- `connections.c` (7 uses);
- `saveload.c` (6), `serialize.c` (3), `engine.c` (3), `datetime.c`,
  `scan.c`, `dcf.c`, `source.c`.

GNU R runs every `cend` between the current and the target context
*before* it jumps (`R462/src/main/context.c:215-228`, `R_jumpctxt` →
`R_run_onexits`). If robigo's trampoline catches first, the records
lived in the frames the `longjmp` discarded, and `cend` never runs (or
runs on dead stack memory). Contexts are also two chains in the doc: the
`RCNTXT` chain (§6) and core §5's shim stack.

**Fix (replacement for the "C ↔ Rust ↔ C" bullet):**

> One context chain: the `RCNTXT` chain. A shim jmp record is a field of
> the `RCNTXT` that owns it (trampoline contexts are `CTXT_CCODE`
> records). Every exit, whether a Rust `Err` reaching an export wrapper or
> a C `Rf_error`, runs `R_run_onexits` semantics before any `longjmp`.
> That means calling `cend` (and `on.exit` for closure contexts) of every
> record above the target, innermost first, with `R_GlobalContext` set as
> GNU R sets it. It does this from the frame that is about to jump. A Rust
> `Err` propagating through Rust frames runs the same walk when it pops a
> C-pushed record. core.md §5 is otherwise kept: only export wrappers jump,
> and the trampolines turn jumps into `Err`.

##### B4. Slot frames have no dynamic bindings, no definition order and no cell kind (§5, §12)

**Problem 1: names outside the shape.** A shape maps the symbols the
compiler saw to slots. Closure frames routinely get names it did not see:
`assign()`, `eval(quote(v <- 1), environment())`, `delayedAssign`,
`list2env(.., envir = environment())`, `makeActiveBinding`,
`sys.function` tricks, `local()`. The doc has no overflow storage.
Astra (`astra-r2.md:132`, `:150`), Fable (`fable-r2.md:545`, `:575`) and
Muse (`muse-r2.md:698`) all had one. The merge dropped it.

**Problem 2: order.** GNU R *prepends* each new binding in an unhashed
frame (`R462/src/main/envir.c:1985-1999`, `SET_FRAME(rho, CONS(value,
FRAME(rho)))`). `function(a, b) { z <- 1; y <- 2; ls(sorted = FALSE) }`
lists `y z a b`, and `rm` followed by reassignment moves a binding to the
front. That order depends on the run, not the shape, so "slot creation
order reproduces unhashed order" (§12) has no mechanism behind it. It is
observable through `ls(sorted = FALSE)`, `as.list(environment())`,
`mget`, `eapply` and `serialize`, and `as.list(environment())` is common
in packages for capturing arguments.

**Problem 3: a stable cell.** `R_findVarLocInFrame` is used by GNU R's
own methods package (`R462/src/library/methods/src/methods_list_dispatch.c:605`;
`tests/capi/symbols.tsv`). The "transactional materialization" (§5) needs
a slot kind that redirects to the materialized cell, or later VM writes
miss it.

**Fix (add to §5's closure-frames bullet):**

> A slot frame has an overflow area for bindings not in its shape (a
> slots object at a fixed offset, null until first use). Each binding
> carries a definition stamp (u32, set when an unbound slot becomes bound,
> never on reassignment). Enumeration lists bindings by stamp descending,
> which reproduces GNU R's prepend order including `rm` and reinsertion.
> Formals are stamped in reverse formal order at call entry, so that they
> list last and in formal order. The slot kinds add `cell`: the slot holds
> a pointer to a GNU R binding cell made for `R_findVarLocInFrame`, and
> every read or write through the slot goes to the cell. The VM checks
> `cell` on the same kind test it already does for `active`.

Add a §12 row: "`ls(sorted = FALSE)` on closure frames with dynamic
definitions, `rm` and reinsertion | definition stamps | conformance case
per path".

##### B5. Which code GNU R would have byte-compiled is observable; the doc drops it (§6, §9, §12)

**Problem.** In GNU R, `BODY` of a compiled closure *is* a BCODESXP.
`print(f)` shows `<bytecode: 0x..>`, `.Internal(inspect(f))` shows the
BCODESXP, `serialize(f)` and `saveRDS(f)` write GNU R's code vector and
constant pool, and `.Internal(disassemble())`, `mkCode` and `bcClose` are
reachable.

Which closures are compiled, and so which ones run `bcEval` semantics
rather than `eval` semantics, is decided by `R_CheckJIT`
(`R462/src/main/eval.c:1569-1631`: `MIN_JIT_SCORE` 50, `MAYBEJIT`/`NOJIT`,
top-level versus nested closures, `R_JIT_STRATEGY`). Base and recommended
packages are compiled at install.

Today's runtime already reproduces all of this:
- `crates/robigo-runtime/src/vm/rjit.rs:37-62` ports `R_CheckJIT` and
  `JIT_score`;
- `print.rs:183` prints `<bytecode: ..>`;
- `tests/r/serialize/bytecode_construct.R` checks serialize bytes against
  GNU R fixtures;
- the CI job `bytecode-fidelity` (`docs/verification.md:409`) covers it.

The doc says "BODY (the source, never replaced)" and "everything is
compiled", and §12 has only "byte-code compile-time binding". With the
doc as written, `print(lapply)` loses its `<bytecode>` line and
`saveRDS(f)` changes bytes.

**Fix (add to §9 "One engine"):**

> A closure runs the `Bc` profile exactly when GNU R would hold a
> BCODESXP for it. That covers installed base and recommended packages,
> `compiler::cmpfun`, and `R_CheckJIT`'s decisions, ported with their
> state bits and environment variables. Otherwise it runs `Eval`. The
> code object is robigo's. A GNU R-shaped BCODESXP (`cmp.R`'s code vector
> and constant pool, `R_bcVersion`) is produced on observation: `print`'s
> `<bytecode>` line, `inspect`, `serialize`/`saveRDS`, `bodyCode`,
> `disassemble`. It comes from an exact port of `cmp.R`'s code generator,
> or by running GNU R's compiler package itself, with the closure's
> source as input. `body()` returns the source as in GNU R
> (`R_ClosureExpr`).

Add §12 rows:

| observable | changed by | mechanism |
|---|---|---|
| `<bytecode>` in `print`, BCODESXP in `inspect` | one engine | `R_CheckJIT` state ported; BCODESXP on observation |
| `serialize` of compiled closures, `mkCode`/`bcClose`/`disassemble` | private code objects | GNU R BCODESXP made by cmp.R codegen; `bytecode-fidelity` fixtures carried over |

##### B6. §2 contradicts itself on package ALTREP reads

**Problem.** Line 59 says calling a package ALTREP method takes
`&mut Cx`. Line 63 says reads (`len`, ...) take no token. But `XLENGTH`
of an ALTREP object calls the class's Length method
(`R462/src/include/Defn.h:408`, `XLENGTH_EX`; `altrep.c:340`
`ALTREP_LENGTH`). For a package class that method is C, which can
allocate, collect or run R code. As written, `len()` on a package ALTREP
inside `'g` is unsound.

**Fix (replacement for the §2 "Reads" bullet):**

> Reads (`len`, `attrib`, `car`, `cdr`, `tag`, `enclos`, `formals`, ...)
> take no token and return `Ref<'g>`, except on package ALTREP objects:
> `Ref::len()` returns `Option<usize>` (`None` for a package class), and
> `cx.xlength(root)` calls the method behind `&mut Cx`. When `Args` is
> built, it records whether any argument is a package ALTREP object, so
> builtins take one branch.

##### B7. D6 offers an inexact variant (§4 vs §15.2)

**Problem.** §4 mandates GNU R's deferred link counts. D6 lists "eager"
as a candidate. Eager counting of stack values makes `MAYBE_SHARED` true
more often than in GNU R, which causes extra duplicates that `tracemem`
and `inspect`'s `REF(n)` show. It cannot win a "fastest exact" contest.
Astra objected to exactly this (round 2: "reject benchmarking unsound
candidates as possible winners").

**Fix.** Delete D6, or reword it as: "D6 (control only, never adoptable):
eager link counting, to measure what the deferred protocol saves."

##### B8. Experiment A: state the result, the meaning of "pin", the decision rule and the split (§3, §15.1)

**Problem 1: the result is in, but the doc still treats the question as
open.** `expA-results.md` shows:
- pin-only passes the planted-stale-word test, which fails without it;
- 0 misses in 30×625 stress runs per tier with pin-only and the plain
  stub;
- +2 instructions per `.Call` for pin-only, −120 (−1.6%) for the plain
  stub;
- no conformance regressions.

**Problem 2: "pin" is ambiguous.** In the code a pinned word is *marked
and traced* (`gc.rs` `scan_stack` ends in `c.trace()`, `:1473`). Pinning
a block without tracing it would free its children under C. The doc must
say "trace".

**Problem 3: the disagreement is hidden.** The doc presents the choice as
a measurement. It is also a values split. Astra and Muse reject pins
because a pinned EXTPTR's C finalizer can free a resource that a buggy
package still uses (`astra-r2.md` positions table; `muse-r2.md` §0.5).
The answer belongs in the doc: GNU R frees such an object *outright*
(`expA-results.md`, last paragraph). The buggy package's next use is
undefined behaviour in both runtimes. For correctly protected packages,
pins are exact and strong roots are not, because stale words delay
finalizers (the 30/0/0 interpreter misses). So pins are at least as
compatible as GNU R everywhere.

**Fix (replacement for §3 step 3 and the body of §15.1):**

> 3. C segments conservatively, **after** step 2: each word that points
>    into an allocated object marks it and traces from it, so its memory
>    and everything it reaches stay allocated; no conservative word
>    decides a weak reference or a finalizer.
>
> §15.1, result (2026-11-13, `docs/ledger/core.md`): pin-only passes the
> planted-stale-word test that fails without it, misses no finalizer in
> 30×625 stress runs per tier with the plain `.Call` stub, and costs
> nothing per call; the plain stub saves 1.6% per `.Call`. Decision: pin
> after weak processing, and drop the stub's narrowing and 1 KiB zeroing,
> once the package suites pass with both switches. Council split recorded:
> Astra and Muse objected that a pinned EXTPTR's finalizer can release a
> resource a package still uses. Answer: that package lacks a PROTECT, and
> GNU R frees the whole object at the same collection, so robigo is no
> less safe; correctly protected packages get exact finalizer timing,
> which strong conservative roots cannot give. `ROBIGO_GC_PIN_AUDIT=1`
> reports every object that a weak reference or finalizer decided dead
> while a C word pinned it (a package bug report).

##### B9. M1 is not specified enough to start (§14.1 M1, §15)

**Problem 1: base.** "Enough base for `calls.R` unmodified" hides a
dependency. `calls.R` calls base R closures: `new.env`, `tryCatch` (and
through it `tryCatchList`, `tryCatchOne`, `doTryCatch` and
`.addCondHands`), `stop`, `structure`, `invisible`, `cat`, `list` and
`UseMethod` (`benchmarks/suite/calls.R:9-28`). Booting base is M3's exit.

**Problem 2: what M1 measures.** D7 needs C API exports (M3/M5 after
B1), D8 needs the heap changes (M2), D11 needs a booted base, and D10
needs `paste`. The doc does not say which D's M1 must decide.

**Fix (replacement for the M1 row's "what" column, plus a column in the
§15.2 table):**

> M1: `crates/robigo-rt`, `robigo-heap` (B1), outside the default build:
> `Cx`, the object API, slot frames with overflow and stamps, closures,
> argument slots, contexts, `RResult`, safepoint collection, the VM with
> both profiles for the ops `calls.R`, fib and binarytrees use, S3
> dispatch, the condition system's internals. It loads the base R
> functions `calls.R` reaches from `rlib/` 4.6.2: the files listed in
> `crates/robigo-rt/proto/base-subset.txt`, at least `conditions.R`,
> `stop`/`warning` from `stop.R`, `structure`, `new.env`, `cat`; the
> loader evaluates only those definitions. The internals they call are
> Rust.

Add a "decided in" column to §15.2:

| decided in | decisions |
|---|---|
| M1 | D1, D2, D3, D4, D5, D9 |
| M2 | D8 |
| M3 | D2u, D10, D11 |
| M5 | D7 |

D6 is deleted (B7).

#### 3. Where the doc misrepresents the council

1. **The C-frame pins split is presented as a measurement question** (B8).
   Opus and Fable were for pins; Astra and Muse were against, on a
   resource-safety argument that no run can settle. Record it and answer
   it.
2. **Allocation: §3 credits "Opus, Fable" with Muse as the fallback, and
   leaves out Astra's third position.** Astra wanted allocation to take
   `&mut Cx` (`astra-r2.md`, "What changed", bullet 2, and the dispute
   table: "Allocation takes `&mut Cx`"). It is dominated by the chosen
   design: it makes every allocation end every `Ref`, which is GNU R's
   PROTECT discipline enforced by the compiler. Still, say so in one line,
   and keep it as D1's third row if E4 fails and Muse's fallback is slower.
3. **The hashed-environment variant was dropped.** Muse (B5, with a
   pre-registered fallback) and Astra (§3: "compare compact index plus
   logical-order metadata against actual chained cells; retain the faster
   exact choice", reproducing `R_HashResize`) both asked for robigo's own
   open-addressing table, with GNU R's order kept as metadata, to be
   measured against GNU R's chained HASHTAB. §5 fixes GNU R's
   representation with no variant. Namespace and global lookups are on
   hot paths. Add **D2h: hashed environments: GNU R chains (default);
   open addressing with GNU R order metadata**, with Muse's rule: any
   unexplained order difference reverts it.
4. **The slot-frame overflow, definition order and stable-location
   mechanisms were dropped.** All three other seats had them (B4).
5. **C-first builtins were one seat's position.** Only I proposed
   C-first. Astra, Fable and Muse wanted re-ports from 4.6.2's C into Rust
   (`muse-r2.md` B2, `fable-r2.md` pick 1(c), Astra's migration row). L4
   makes the crutch a lead decision, which settles it. But Fable's
   objection to the reverse bridge carries over: ratchet marks earned by
   GNU R's code are not robigo's (`fable-r2.md` pick 1(d)). Add to §10:
   "each `rt` ratchet case records the C builtins on its path (registry
   counters), so 'ratchet to zero' is measured in cases as well as in
   builtins".
6. **Muse's header-surface parity was dropped.** robigo-rt's installed
   headers must declare exactly 4.6.2's API. Packages that fail on GNU R
   4.6.2 because `VECTOR_PTR`, `DATAPTR` or `LOGICAL0` are gone must fail
   here too (`muse-r2.md` §0.4). This belongs in §12 and §14.2.
7. **Fable's `#[derive(Trace)]` for isolate state was dropped**
   (`fable-r2.md` C6). It turns "a forgotten root in `Cx`" into a compile
   error. §11 lists the isolate fields but not how they are traced.
8. **Astra's "a dead value in a registered slot delays a finalizer" is
   credited correctly** (§3). Fable's liveness maps and Astra's
   clear-at-last-use are both D9. This one is fine.

#### 4. Answers to §17

**Q1. Can any of `rlib/` stop being vendored?** Not as *data*. Every base
closure is observable: `print(f)`, `body`, `formals`, `deparse`,
`identical(body(f), quote(..))`, `trace`, `getAnywhere`, and serialized
bytecode (B5). Any representation that reproduces those *is* GNU R's R
source. But three things can go:

1. **The checked-in copy.** Fetch `R-4.6.2.tar.gz` by SHA-256 at build
   time and extract `src/library/*/R`. That is a build input, as the
   oracle image already is, not a vendored tree.
2. **Parsing at startup.** At build time, produce a lazy-load image of
   each base namespace: closures with GNU R's bodies, plus robigo's code
   objects for the `Bc` closures. Startup then maps the image. GNU R does
   the same with `base.rdb`. Base boot cost (421M instructions for
   `--vanilla -e 1` today, ledger core 2026-11-10) mostly disappears.
3. **Top-level setup code** that is not a closure body: assignments,
   `environment<-` calls, `.Machine` building, the `zzz.R` hooks. Only
   the bindings it produces are observable, so Rust may produce them.

The hottest closures (`match.arg`, `stopifnot`, `inherits`, `vapply`
wrappers, `seq`, `do.call`) get intrinsics that are guarded on the callee
*being that exact closure object* and on its debug and trace flags
(`RDEBUG`/`RSTEP`; `trace()` replaces the binding, so the identity guard
covers it). They are never replacements.

**Q2. A third option for C frames?** No better option exists, and
experiment A makes the case. Pins that trace, decided after weak
processing, are exact for every correctly protected package. Strong
conservative roots are not: stale words delay finalizers (30/0/0 misses
with the stub off). For an unprotected object, pins are no less safe than
GNU R, which frees it outright. The two refinements worth having:

- `ROBIGO_GC_PIN_AUDIT=1` (B8) turns the remaining risk into package
  bug reports.
- Base natives and the crutch are GNU R's own PROTECT-correct code, so a
  CI mode could treat their segments as *not scanned at all*, which
  proves that correctness under gctorture. Robigo keeps pins only for
  package frames in production. That tests our assumption about GNU R's
  code without changing behaviour. *(guess: worth one CI job, not a
  production mode)*

**Q3. Designed away versus ported.**

- **Owned by the evaluator, and so Rust from M3** (about 176 entries, B2):
  everything in `eval.c`, `envir.c`, `errors.c`, `context.c`, `debug.c`,
  `memory.c` and `objects.c`. That is control flow, assignment,
  `function`, `return`, `missing`, `nargs`, `sys.*`, `parent.frame`,
  `on.exit`, `UseMethod`/`NextMethod`, `standardGeneric`, the condition
  internals, `gc`/`reg.finalizer`, `environment` and the binding
  functions.
- **Designed away** (VM/JIT intrinsics plus a Rust generic fallback; port
  these first):
  - the arithmetic, comparison and logic groups, `&&`/`||`;
  - Math group scalars;
  - `[`, `[[`, `$` and their replacement forms on unclassed vectors,
    lists and environments;
  - `length`, small-arity `c`, `list`, `:`, `seq_len`, `seq_along`,
    `rep_len`, `vector`/`numeric`/`integer`/`character`/`logical`;
  - the `is.*` type predicates, `is.na` on scalars, `inherits`, `class`,
    `oldClass`, `attr`, `names`, `.subset`/`.subset2`, `invisible`,
    `identical` on scalars;
  - `sum`, `max`, `min` and `prod` on short vectors;
  - `lapply`, `vapply`, `Map`/`mapply` and `Reduce`'s inner call loop,
    run as a VM loop that calls closures without re-entering a builtin.
- **Ported as functions** (a kernel, no call-site benefit):
  - `sort`/`order`/`rank` (`radixsort.c`, `sort.c`);
  - `unique`/`match`/`duplicated` (`unique.c`);
  - the string family (`paste`, `format`, `sprintf`, `formatC`, `nchar`,
    `substr`, `strsplit`, `grep`/`regexpr`, `chartr`), through
    `robigo-text`;
  - `print`/`deparse`/`cat`/`format.info`;
  - `array.c` (`aperm`, `%*%`, `matrix`), `bind.c` (`cbind`, `rbind`,
    `unlist`);
  - `seq.c`, `summary.c` in full, `cum.c`, `random.c` (over
    robigo-numerics), `datetime.c`.
- **Ported last, with no speed motive:** connections, `serialize`,
  `saveload`, `scan`, `dcf`, `platform.c`, `sysutils.c`, `internet.c`,
  `engine.c`/graphics. These are mechanical ports, and the oracle is the
  differential mode.

**Q4. What §12 is missing:**

- GNU R's JIT decisions and BCODESXP observables (B5).
- Dynamic order of unhashed frames (B4).
- **`Rprofmem` output** (sizes of large allocations and "new page"
  events) and the **"cannot allocate vector of size …"** message. Both
  must use GNU R's size formulas, not the real ones under a 24-byte
  header and per-type nodes.
- **`Rprof` stack samples.** Function names come from the context chain;
  inlined pure closures and intrinsics must still appear as frames.
- **`sys.function`, `sys.call` and `parent.frame` *during* the forcing of
  a promise-free argument** (the forcing pushes no function context in
  GNU R; delayed-slot forcing must match).
- **Header surface** (installed headers equal to 4.6.2's declared set;
  section 3 item 6).
- **`tracemem` on argument slots.** A forced slot must contribute exactly
  the REFCNT that a PROMSXP plus PRVALUE would. Materialization moves the
  count and does not add one.
- **`environmentName`, `format(env)` and `print(env)` for slot frames,
  and `env.profile()`** (reports hashed-table stats, depends on D2h).
- **The browser and `debug`/`debugonce`/`trace` on closures whose calls
  were inlined.** `RDEBUG` must deopt or prevent the inlining.

**Q5. Are the §13 bars right?** They are right in kind. Five gaps:

1. **Rivals by wall time only.** Instruction counts are meaningless for
   FastR (JVM) and Ř (LLVM JIT) because of compilation and GC threads.
   Measure instructions against GNU R and wall time against everyone.
   Write a noise rule for "fastest on every benchmark" (for example,
   ahead by more than the 95% CI of 10 runs, else a tie that counts as
   not fastest).
2. **Missing workload: package load time** (`library(dplyr)`,
   `library(data.table)`, `library(tidyverse)`): namespace loading,
   lazy-load, S4 and methods setup. It dominates short scripts and CI
   jobs.
3. **Missing workload: GC latency**: p99 pause and total GC time on the
   package suites and a long allocation-heavy run, beside peak RSS.
4. **Missing workload: C API overhead**: per-export cost (`REAL`,
   `VECTOR_ELT`, `Rf_eval` callback, `R_UnwindProtect`) against GNU R,
   for Rcpp, cpp11 and vctrs-style code. The package suites mix it with
   everything else.
5. **Missing workload: I/O and serialization**: `readRDS`/`saveRDS`,
   `read.csv`, `readLines`, `gsub` over 1e6 strings. S4 and R5/R6 dispatch
   also deserve named rows (R6 is the reason for D2u).

#### 5. Non-blocking improvements, ranked

1. **Make immortal constants process globals**, not per-isolate
   accessors: `R_NilValue`, `R_NaString`, `R_UnboundValue`,
   `R_MissingArg`, `R_BlankString`, the logical scalars. The crutch reads
   them in inner loops, and the appendix already wants static immortal
   objects. Per-isolate macros stay for mutable globals (`R_GlobalEnv`,
   `R_Visible`, `R_PPStackTop`, `R_GlobalContext`).
2. **`ROBIGO_GC_MOVE=1`'s pin rule.** "Objects ever handed to C are
   pinned" becomes "almost everything" once the crutch is in, because
   every object crossing a C builtin counts. Instead, pin what C can still
   name: objects marked through conservative words in that collection,
   the precious set, and vectors whose data pointer was handed out
   (`DATAPTR`/`REAL`), sticky per object.
3. **The `FreshVec` no-barrier invariant must survive M8.** It is sound
   only while fresh objects are young or the collector is not
   generational. State it now: "any generational design keeps fresh
   objects in the young space, or `FreshVec` publishes through the
   barrier", so M8 doesn't silently break it.
4. **`RResult<Ref>` is one word only if `Ref` wraps `NonNull`.** Say so,
   and add a size assertion.
5. **Say where `RCNTXT` records for R calls live:** in the VM stack's
   reserved range, embedded in VM frames, so their addresses are stable
   and closure calls need no Rust recursion.
6. **Crutch call overhead as an exit metric.** Each C builtin call from
   the VM builds a pairlist and goes through a trampoline. Add to M3:
   "instructions per call of the 20 most-called C-served builtins,
   reported. Any builtin above 1.2x GNU R moves to Rust before M7."
7. **The symbol arena under several isolates** (core.md §10): install is
   concurrent. Pick a lock-free append arena or a per-process lock, and
   say which.
8. **E1's rival bar** compares a JIT-off prototype against warmed FastR
   and Ř. Keep it, but state the mode (one process) so that JVM startup
   is not the reason it passes.
9. **§2's `Args` for specials:** say what a SPECIALSXP receives (the
   unevaluated LANGSXP arguments and `env`), and that `Args` covers only
   builtins.
10. **§14.2: record which CRAN package versions** the 4.6.2 package
    suites use, so that a package update is not mistaken for a runtime
    regression.


### Muse

### Council round 3 review of `docs/g3.md` (Muse Wonder Full)

Reviewer: Muse Wonder Full, round 3. Scope per `brief-r3.md`: review the
merged G3 design only; read-only; no new design. `expA-results.md` was not
present when I reviewed (only `expA/stress.R` and the in-progress
`ROBIGO_GC_PIN_ONLY`/`ROBIGO_CALL65_PLAIN` diff in the worktree), so §15.1
is reviewed as designed. Paths: repo paths are worktree-relative; `R462/`
is the GNU R 4.6.2 tree in the scratchpad (`r462/R-4.6.2`); council files
are the round-2 designs in this directory. Guesses are marked *(guess)*.

#### 1. Score: 5/10

Sub-scores, because the two halves of the question diverge:

| aspect | score |
|---|---|
| Direction (one heap, plan B, safepoint collection, GNU REFCNT, 4.6.2, measurement gates) | 8/10 |
| Correctness of the doc as a merged, self-consistent spec | 5/10 |
| Specific enough to start M1 without inventing mechanisms | 4/10 |

The strategy is the council's convergent answer and it is the right road
to "fastest R runtime, fully compatible". But as written the doc is a
decision record, not an implementable spec: at least six load-bearing
mechanisms (root representation, `Cx` re-entrancy, slot-frame growth and
encoding, GNU bytecode, the C-builtin crutch's ABI, heap-crate
coexistence with the frozen runtime) are missing, picked inconsistently
from different round-2 designs, or contradicted by another section. An
M1 implementer would have to mine four conflicting round-2 files and
guess which parts are normative. That is exactly how the two-object-model
bugs G3 exists to delete get re-invented. Fix B1–B9 and this is an 8.

#### 2. Blocking findings

##### B1. §2/§11: `&Cx`/`&mut Cx` branding is unsound as stated under re-entrancy

Problem. Three compounding gaps. (a) Allocation "takes `&Cx`" while the
heap is embedded in `Cx` (§11): allocation must mutate the heap through
interior mutability, so "allocation never collects" is a dynamic
invariant inside `unsafe` core code, not something "the borrow checker
proves" (§2 table). Today allocation is `&mut Core` and collects when
due (`collect_if_due` → `gc_internal` → `collect_needing`,
`crates/robigo-core/src/gc.rs:571-590`; `alloc_object` at `gc.rs:604`
is `&mut self`). The appendix does not even
list the poll-only allocator split as a core change. (b) A Rust builtin
holding `&mut Cx` that calls C (package ALTREP, `.Call`) suspends that
`&mut`; the C code re-enters through an export, which must manufacture a
second `&mut Cx` from the isolate pointer (§11). Two live `&mut`s to the
same record is precisely the aliasing today's code already has to
disclaim in prose: "the API re-enters itself through callbacks, so the
`'static` here is the caller's promise, not the compiler's"
(`crates/robigo-capi/src/state.rs:124-142`). (c) Nothing states the
minting invariant that makes `Ref<'g>` dereferenceable: every `Ref`'s
object is reachable from a precise root, or was allocated after the last
safepoint and is rooted/stored before the next one.

Smallest fix (replace the §2 layer table's safety column and add after
the "`&mut Cx` ends every `'g`" bullet):

> `Cx` is a capability over `*mut Isolate`; its fields live in
> `UnsafeCell`s. `&mut Cx` is a short-lived reborrow token: every entry
> into R or C code consumes it and re-creates it from the raw pointer on
> return, so no Rust reference into the isolate is live while foreign
> code runs (the discipline `crates/robigo-capi/src/state.rs:124` states
> today). The borrow checker then proves: in the safe layer, no `Ref` or
> slice outlives a safepoint. It does not prove that allocation never
> collects (an audited core invariant, checked by `ROBIGO_GC_MOVE=1`),
> nor anything inside the raw layer. A `Ref` may be minted only from a
> `Root`, `Args`, `Global`, a field of an object so rooted in the same
> interval, or a fresh allocation in the same interval.

And add an appendix row: "`gc.rs` allocation: split into a poll-only
fast/slow path that sets `GC_DUE` and never calls `collect_needing`;
collection entered only from `Cx::safepoint` and allocating C exports."

##### B2. §2/§3: what a `Root` *is* is undecided — three incompatible round-2 answers merged by name only

Problem. §2 names `Root<'s>`, `Out<'s>`, `Args<'a>`, `Global` and §3 lists
what marking scans, but never says where root slots live, whether their
addresses are stable, who saves/restores the top on dispatch and unwind,
or how a `Root` survives being stored in a Rust struct. The round-2
answers conflict on a safety-relevant point: Opus pushes roots onto
GNU R's protect stack (`opus-r2.md:244,323`); Fable and Muse put them in
a separate value/root stack precisely because "C may `UNPROTECT` across a
callback" (`fable-r2.md:139,202-203`; `muse-r2.md:630`). If Rust roots
share `R_PPStack`, a C callback's miscounted `UNPROTECT(n)` — or the
`R_PPStackTop` restore in context unwinding (`R462/src/main/context.c:171`
area, `R_restore_globals`) — silently un-roots live Rust values. §3's
mark list says "protect stack" and "VM stack" but no Rust root stack,
so as written a builtin's `Root`s are not in the mark list at all unless
they alias one of those.

Smallest fix (add to §2, and name it in §3's list as item 1):

> Roots live in the isolate's value stack (reserved address range, §9),
> never on `R_PPStack`: C may `UNPROTECT` across a callback and context
> unwinding restores `R_PPStackTop`. `Root<'s>` is an index into that
> stack branded by a scope; slot addresses are stable because the range
> is reserved, not grown. Every builtin/native dispatch and every unwind
> saves and restores the stack top; a scope's roots die when its top is
> restored, on every path including `Err`.

##### B3. §3/§15.3 E4: no mechanism bounds deferred garbage inside one builtin or one JIT loop

Problem. Safepoints are listed at R entries, VM back-edges/calls, builtin
*return*, explicit `cx.safepoint()`, and allocating C exports. A single
Rust builtin that allocates unbounded intermediates before returning —
`paste`, `c`, `unlist`, coercions, `as.character` on a large vector, the
`alloc_rooted` large-result path's own fill loop — collects nothing until
it returns, so E4's "never above max(32 MiB, half the live heap)" fails
by construction on ordinary inputs unless every such builtin polls. The
doc gives no polling rule, no byte budget, and no list of kernels that
must poll; the same hole applies to interrupt latency in a long Rust
kernel (the poll is "merged with the interrupt poll" only in the VM).
Related overclaim: "C keeps GNU R's collect-at-any-allocation behaviour"
(§3 item 2). Under the poll design an allocating export collects *when
due, on entry, before allocating* (Fable's actual design,
`fable-r2.md:492-497`); GNU R collects inside the allocating call
(`R462/src/main/memory.c` `allocSExp`/`allocVector3` paths). The
observable difference — which unprotected C locals die, and when
`gctorture(TRUE)` fires relative to the fresh result — needs a stated
equivalence argument or a torture test, not an assertion.

Smallest fix (add to §3 after item 3):

> Any Rust loop whose allocation is not bounded by a constant must call
> `cx.poll_alloc(bytes)` per iteration-chunk; the allocator's slow path
> sets `GC_DUE` and `poll_alloc` safepoints with its inputs in roots and
> slices reacquired (the §2 kernel rule). The chunk bound is part of
> E4's measurement; a builtin that cannot name its chunk does not merge.
> C exports collect on entry when due, before allocating, with their
> `SEXP` arguments protected; this differs from GNU R's
> collect-inside-allocation only for objects reachable solely from
> unprotected C locals, whose fate experiment A decides (§15.1).

##### B4. §5: slot frames have no growth mechanism, no slot encoding, no listing-order mechanism

Problem. "One variable-size ENVSXP with inline typed slots" cannot be
built as written. (a) R frames are open: `assign`, `eval(quote(x <- 1),
env)`, `list2env`, `<<-` creating in an enclosing frame, and `rm` +
redefine all add bindings a compile-time shape cannot know. A
variable-size allocation sized by the shape has nowhere to put them, and
the object cannot move (its address is its identity). All three designs
that specified frames had an overflow path — Fable's overflow slots
object and shape transitions (`fable-r2.md:545,571-577`), Muse's
extended-frame promotion (`muse-r2.md:696-698`), Astra's overflow block
(`astra-r2.md:132,150`) — and G3 dropped every one. (b) A delayed slot
needs expression + environment + PRSEEN + MISSING + counted-edge state
(fable-r2.md:551-560's key-bit encoding, incl. the thunk-index-in-key
trick that avoids a second object); §5's "kind bits and a value that is
a pointer or an unboxed scalar" has no room for half of that and says
nothing about which words the tracer traces per kind. (c) `ls(sorted =
FALSE)` order for unhashed frames is newest-first with call-frame
arguments in formals order; with preallocated slots, definition order is
data-dependent and must be recorded (fable-r2.md:578-584's rank). §12's
"slot creation order reproduces unhashed order" assumes slots are
created in definition order, which preallocation contradicts. (d)
Reading an unboxed slot as an object must box-and-store-back
(`R_expand_binding_value`, fable-r2.md:585-588) or `get`/`as.list`/C see
different identities per read — §276's bug class. (e) The `R_varloc_t`
sentence: in 4.6.2 it is `{ SEXP cell }` (`R462/src/include/Defn.h:2052`)
and `R_SetVarLocValue` writes through the stored cell later
(`R462/src/main/envir.c:1125-1128`), so "materialized transactionally"
must mean the binding becomes cell-backed permanently, or robigo's
`Defn.h` redefines the struct (Fable's `{env, slot}`, fable-r2.md:589).
As written it can be read as a temporary copy, which loses C writes.

Smallest fix: declare `fable-r2.md` §3.1–§3.2 normative for frame layout
(word map, key bits, overflow, shape transitions, order rank,
expand-on-read, varloc), list G3's deviations from it, and change §12's
row to "listing order from the shape's recorded definition ranks; hashed
envs are GNU R's representation". If the council prefers a different
overflow design, it must be written at the same level of detail before M1.

##### B5. §6/§9: GNU bytecode (BCODESXP) is missing, and "BODY (the source, never replaced)" is false

Problem. The doc never mentions BCODESXP. In GNU R, `compiler::cmpfun`
and lazy-load databases replace a closure's BODY with a BCODESXP
(`R462/src/main/eval.c:1166` evaluates it; `:2367` dispatches to
`bcEval`); base and installed packages arrive that way from `base.rdb`.
Today's runtime has a whole apparatus for this: portable BCODESXP data
(`crates/robigo-runtime/src/value.rs:946-960`), "as bcClose makes it"
(`value.rs:964`), and — worse — call-count-dependent semantics:
GNU R byte-compiles a closure on its first or second call and freezes
the base "language functions" it inlines *at that moment*, so rebinding
`+` between call 1 and call 2 changes behaviour
(`crates/robigo-runtime/src/vm/rjit.rs:1-37`, porting
`eval.c:R_CheckJIT`). §9's one phrase "compile-time binding of base
functions" and §6's "BODY (the source, never replaced)" paper over all
of it. Nor does the doc answer: does `robigo-rt` run `cmp.R` (it is base
R code calling `.Internal(bcClose)` etc., producing real BCODESXPs that
must then be executed), or natively compile with the same *decisions*?
What prints for `.Internal(disassemble(f))`? What does
`identical(f, g, ignore.bytecode = FALSE)` compare
(`R462/src/main/identical.c:314-323`)? When is the §6 code object
invalidated — `body(f) <-`, `formals(f) <-` (the shape key!), C
`SET_BODY` through base natives, `duplicate()` of a closure?

Smallest fix (new §9 bullet + §6 correction):

> BODY may be a BCODESXP (from `cmpfun`, lazy-load databases, saved
> closures); its first constant is the source. `robigo-rt` keeps the
> BCODESXP object (for `identical`, `disassemble`, serialization) and
> translates it to the shared IR on first call, as today's
> `closure_body` does. The `Bc` profile is GNU R's compile *decisions*,
> not its bytecode: the code object carries the per-closure JIT state
> (when GNU R would compile, which language functions and safe internals
> were bound to base then), porting `eval.c:R_CheckJIT` and
> `cmp.R:findCenvVar` (today: `vm/rjit.rs`). Any write to BODY, FORMALS
> or CLOENV — R-level or C — drops the code object and the frame shape;
> `duplicate` of a closure drops the code object.

##### B6. §10: the GNU C crutch is unbuildable as described, and the cited precedent does not support it

Problem. Compiling GNU R's `src/main` builtin files "against robigo's
`Defn.h`, as `scripts/build-natives.sh` does for base-package natives" is
a false analogy, in four ways. (a) The precedent is about something
else: `build-natives.sh` builds base *packages'* native code (stats,
utils, …) as shared objects against the libR API (`scripts/build-natives.sh:1-10`);
the measured win (fastr 30,576 vs 30,311) is native `stats.so` vs Rust
ports called through `.Call`/`.Fortran` (`docs/ledger/core.md:1896-1904`).
`src/main` files are not libR clients; they *are* libR, and they call
each other's internal functions and assign its globals. (b) The ABI is
wrong for §2/§6: every `do_*` has signature `(SEXP call, SEXP op, SEXP
args, SEXP env)` with `args` a pairlist (`R462/src/main/names.c:57`), and
specials evaluate it themselves (`R462/src/main/subset.c:657`
`eval(CAR(args), rho)` is one of hundreds). Serving a C builtin from the
§2 `Args` window therefore requires materializing a pairlist — and, for
evaluated builtins, promise-wrapped or value cells in GNU R's exact
shape — on every call. That is a per-call reverse bridge for the exact
path §6 says allocates nothing, and it reintroduces PROMSXPs the design
just abolished. (c) The internal surface the evaluator must export to
make those files link — `eval`, `findVar`/`defineVar`, `DispatchGroup`,
`forcePromise`, `errorcall`/`warningcall`, writable globals such as
`R_Visible` (assigned at e.g. `R462/src/main/builtin.c:1067`) — is
nowhere inventoried, and §11's per-isolate macro globals cannot serve
unedited C that takes their addresses or declares them `extern`. (d)
Error handling: a `do_*` calls GNU R's `error()`, which longjmps. §6
says "only export wrappers jump", so every C-builtin call from the VM
needs its own trampoline — the cost Opus's own proposal said to measure
(`opus-r2.md:155`, "cost of a C builtin call from the VM
(trampoline)") — and §15 has no such measurement, nor Fable/Muse's
gate for the whole idea: count the distinct builtins base booting
actually needs (`fable-r2.md:1092-1095`, guess <100; `muse-r2.md:85`,
skip the crutch under 150). M3's "base boots … with GNU R's C builtins
where Rust is missing" is load-bearing on all four unknowns.

Smallest fix: demote the crutch from assumed to decided-by-measurement.
Add a decision row D0 (before M3): "count distinct builtins in a base
boot on the old runtime; prototype the pairlist adapter + per-call
trampoline for three files (`arithmetic.c`, `subset.c`, `coerce.c`);
measure instructions/call vs the Rust ports. Adopt `robigo-cbase` only
if the boot set is large *and* the adapter tax is under the E1 bars;
otherwise port the boot set to Rust directly and delete §10's cbase
paragraphs." And correct the precedent sentence to name `stats.so` for
what it is: a package native behind `.Call`, not a `src/main` builtin.

##### B7. §10: differential mode is unsound for effectful builtins

Problem. "`ROBIGO_BUILTIN_DIFF=1` runs both implementations of every
builtin that has both" — in one process, on live state. For anything
with side effects the second run is not the same computation: `assign`,
`rm`, `options`, `set.seed`/RNG state, connections, `parent.frame`
writes, active bindings, finalizer registration, graphics state, and
warning handlers (a `withCallingHandlers` handler would run twice; a
muffling restart taken in run 1 changes run 2). It would report false
differences on correct ports and, worse, teach people to ignore it.

Smallest fix (replace the differential-mode bullet):

> **Differential mode:** builtins are classified (in the generated
> registry, from `names.c` plus a hand list) as value-only or effectful.
> Value-only builtins run both implementations in-process and compare
> result, attributes, warnings and errors. Effectful builtins are
> compared out-of-process: the harness serializes the inputs, runs each
> implementation in a fresh process, and compares stdout/stderr/status
> and a serialization of the returned value and touched environments.
> An unclassified builtin is effectful.

##### B8. §14/appendix: `robigo-rt` cannot share `robigo-core` with the frozen old runtime on main

Problem. L1/L2 put `robigo-rt` on main beside a frozen
`robigo-runtime` + bridge ("CI upkeep only, no fixes"), while M2 and the
appendix change `robigo-core` under both: mark order, "no Rust scan
segments", new trace cases, and — as a measured variant — a 24-byte
header and per-type node sizes. The bridge exists on today's layout,
HOST/hold/twin machinery and conservative segments
(`crates/robigo-core/src/gc.rs:1109-1137`); any layout or rooting change
breaks the frozen runtime on the same main, and L2 forbids fixing it.
Fable's plan-B amendment existed for exactly this reason — fork the heap
crate (`robigo-heap`), because "the layout changes … cannot coexist
with the bridge" (`fable-r2.md:71-76,1054-1055`) — and G3 dropped it
without naming a substitute (no fork, no feature-gated layout v2).

Smallest fix (add to §14.1 before M1):

> `robigo-rt` does not build against `robigo-core` as-is: M1 forks the
> heap into `crates/robigo-heap` (layout, trace, collection policy free
> to change), and `robigo-core` is frozen with the old runtime under L2.
> The flip (M7) deletes `robigo-core` and renames `robigo-heap`. Shared
> fixes during coexistence are ported by hand and listed; there is no
> dual-layout core.

(If the lead prefers a feature-gated v2 layout inside one crate, that
must be the written decision instead — silence is the bug.)

##### B9. §14/§15.3: M1's scope contradicts its exit criteria

Problem. M1's deliverable list (slot frames, closures, argument slots,
contexts, VM subset) excludes S3 dispatch and conditions, which §14
assigns to M3 — but M1's exit is "`calls.R` unmodified" under E1, and
`calls.R` runs `UseMethod`, `tryCatch`/`stop`, `<<-`, environment-method
calls and defaults in one loop (`benchmarks/suite/calls.R:8-29`: `area`
at :17, `safe`/`tryCatch` at :21). Either M1 silently includes most of
M3's evaluator, or E1 cannot be evaluated. Same for E6, which requires
capi cases (`eval`, `pairlist`, `unwind`, `extptr`) "with no bridge" in
the prototype, while M5 is "capi over `robigo-rt`".

Smallest fix: split E1 into the idiom subsets each step owes — M1:
plain, defaults, named, `e$f`, `<<-`; M3 adds S3 and `tryCatch` idioms —
and move "a minimal capi harness sufficient for the four E6 cases" into
M1's scope explicitly (it is the prototype's best early proof that the
object API survives C).

##### B10. §3/§15.1: experiment A as designed cannot decide pins vs strong roots

Problem. *(Results file absent; reviewed as designed, plus the probe and
flags found in the worktree.)* The only stated outcome metric is the
finalizer stress script, which counts finalizers that *did not run*
(`expA/stress.R`: "Prints the (a, b) pairs whose finalizer did not run").
Pin-only marking wins that metric by construction: deciding weak refs
and finalizers before the conservative scan can only make finalizers
run *more*. The cost side of the trade — a finalizer running *too
early*, while C code still holds the object through an unprotected
local, freeing the external resource under a live pointer (Astra:
`astra-r2.md:17,113`; Muse: `muse-r2.md:70`, "the finalizer frees the
C resource under a live pointer — memory stays valid, the resource does
not") — has no probe at all, so the experiment can return "adopt pins"
without ever testing the objection. Three further design faults: no
pre-registered decision bar (contrast §15.2's own rule, "bars written
before the runs"); `ROBIGO_CALL65_PLAIN=1` conflates two interventions
(stub narrowing *and* 1 KiB zeroing) in one flag, so if plain loses we
learn neither's contribution; and it runs on today's bridge heap, where
C frames hold HOST proxies and precise holds, not on G3's direct-SEXP C
frames — external validity to the G3 question is assumed, not argued.

Smallest fix (extend §15.1):

> Two probes, not one: (i) the missed-finalizer stress (delay cost);
> (ii) a premature-finalizer probe: C code keeps an EXTPTR in an
> unprotected local across an allocation, then uses its resource; the
> finalizer poisons the resource and any use-after-finalize is a hard
> failure. Plus the package-suite delta (which suites change under
> pin-only). Narrowing and zeroing get separate flags. Decision rule,
> written now: pins are adopted only if (ii) fails no suite that passes
> today *and* (i) misses nothing; the stub goes only if its instruction
> cost is measurable and (i)/(ii) are unchanged without it. The result
> is recorded as deciding behaviour for *incorrectly* protected C only;
> correctly protected code is exact under both.

##### B11. §6: a single pending-exit slot loses nested exits

Problem. "`RResult<T>` is one word … the pending exit … lives in the
isolate as a precise root" (singular). GNU R keeps the in-flight jump's
value in `R_jumpctxt`'s own frame across `R_run_onexits`, which *evaluates
arbitrary R code* (`R462/src/main/context.c:215-232`, onexit eval at
`:120-160`): that code can raise and catch its own error, signal a
warning whose handler returns, or invoke a restart — each writing the
single pending slot — after which the outer jump must resume with its
original target, mask and value. Astra raised exactly this ("A single
overwritable pending slot is insufficient: `on.exit` and cleanup code
can catch a nested error while the original exit still exists",
`astra-r2.md:186`) and G3 adopts Fable's single `Pending` without
answering it.

Smallest fix (replace the `RResult` bullet):

> `RResult<T>` is one word: the error type is a token indexing an
> isolate-owned stack of pending-exit records (kind, target context,
> mask, value, condition, call), each a precise root. Running
> `on.exit` code, a calling handler or a restart pushes/pops records;
> an exit resumes only with its own record. Unwind continuations own
> their record.

##### B12. §2/§8: "reads take no token" is unsound for package ALTREP; `regions()` cannot yield slices for generated sequences

Problem. §2 puts `len` among token-free reads and says "`Length` never
forces elements" — true for robigo's classes, false in general:
`ALTREP_LENGTH` dispatches to the class's C Length method
(`R462/src/main/altrep.c:340-343`), which can allocate, error and
longjmp; same for `Dataptr`. Astra's rule — "Generic ALTREP Length and
Dataptr_or_null are effectful even if their names sound like reads"
(`astra-r2.md:91`) — was dropped. Under the §2 API a safe builtin could
hold a `&'g [T]` or a `Ref<'g>` across a package method that runs R
code and collects. Separately, `regions()` "for robigo's compact
classes without materializing" cannot return `&'g [T]` slices for a
sequence: there is no storage to slice; GNU R's `ITERATE_BY_REGION`
fills a caller buffer. The API shape needs the buffer.

Smallest fix (amend §2's reads/vector bullets):

> Token-free reads are header and node fields of non-ALTREP objects and
> of robigo's own ALTREP classes (whose methods are Rust, allocation-
> only and proven non-signalling). Length, dataptr and element access
> on any other ALTREP take `&mut Cx` on a root. `regions()` yields
> slices only for materialized storage; for generated classes it is
> `regions_into(&mut [T; N])` filling a caller buffer, GNU R's
> `ITERATE_BY_REGION` shape.

##### B13. §6: the argument/binding state machine is incomplete where GNU R's is now public

Problem. Four holes in two paragraphs. (a) Incoming promises: an
argument can *be* a PROMSXP (forwarded through `...`, made by C,
`delayedAssign`), and promises form chains — GNU R's binding-type query
unwraps them (`R462/src/main/envir.c:840-856`, `promiseUnwrap`). G3's
slot kinds (value/missing/missing-default/delayed/forced/active/locked)
have no chain rule and no "value holding a PROMSXP" case (Fable's table
had it: `fable-r2.md:694`). (b) Transitions: assigning to a forced
argument (`x <- 2` in the callee) turns a Forced binding into a Value
binding in GNU R's derivation; forcing a delayed slot from C
(`R_forcePromise`-equivalents, `findVar`) must move Delayed→Forced with
the expression retained. No transition table exists, yet the 4.6 API
makes every state queryable. (c) Memoized argument matching must replay
per-call side effects: partial-match warnings are signalled on *every*
call when `options(warnPartialMatchArgs)` is set
(`R462/src/main/match.c:277-279`), and the option can change between
calls; a cached permutation that skips `matchArgs` skips the warning.
(d) §4 says to port `unpromiseArgs`, but in the slot design there are no
argument promises to unpromise — the REFCNT adjustments it performs
(forced arguments stop counting their promise's references) need a
stated slot-world equivalent, or §4 and §6 contradict.

Smallest fix: add a state table to §6 (rows: slot kinds incl.
value-holding-PROMSXP and chain unwrapping; columns: force, assign,
`rm`, C binding-API query, materialize — each cell the resulting kind
and REFCOUNT change, each row cited to `envir.c`/`eval.c`), and a
sentence: "A memoized match plan records which partial matches it used;
executing the plan signals the same warnings `matchArgs` would, under
the options in force at that call."

##### B14. §11/§5 vs `docs/core.md` §10: the sole-isolate global and the symbol arena contradict a lead decision, and measurement cannot settle either

Problem. D7 offers a "sole-isolate global" as a *measured* variant while
noting core.md §10 forbids process-global mutable state. §10 is not a
taste preference: Decision A.2 (lead requirement, 2026-10-28) mandates
multiple isolates per process, one thread each, with all mutable state
behind one thread-local pointer (`docs/core.md:977-986`). A faster sole
global does not become compliant by winning a benchmark; either the
lead amends A.2 or D7's second variant is out. Same section: A.2 puts
symbols in *one shared* table (immutable once created, interning under
a lock) with only the symbol's value cell per isolate
(`docs/core.md:1000-1002`), because the frozen/shared segment and
cross-isolate immutable data reference symbols. G3 §5's "immortal
objects in a 64-byte-aligned arena; its id arena arithmetic" plus §3's
"per-isolate symbol cells" never says whether the arena is shared or
per-isolate; per-isolate arenas give the same symbol different ids and
addresses per isolate and break A.2's sharing, a shared growing arena
needs the lock and a no-reallocation rule for id arithmetic to mean
anything.

Smallest fix: replace D7 with "TLS (initial-exec where the platform
allows) is the design; if the lead wishes to re-open A.2 for a measured
sole-global variant, that is a decision to record in §1, not a
prototype outcome" and add to §5: "The symbol arena is process-shared
(one lock, append-only segments, never reallocated; ids are global);
per-isolate state is the symbol's value cell and the base/global cache
tables indexed by the global id."

#### 3. Where the doc misrepresents the council

Ordered by consequence. Cross-references to §2 are the same fault seen
from the merge side; they are listed here because the merge, not only
the mechanism, needs fixing.

**X1. The C crutch is one member's position, merged as consensus, then
changed.** Only Opus proposed compiling `src/main` (B3, with the long
tail *staying* C: "exact by construction for the long tail (~600
internals)", deleting 40–50k lines of Rust ports, `opus-r2.md:155`).
Astra, Fable and Muse all re-port to Rust (Muse explicitly as the
plan: `muse-r2.md:125-136`; Fable: no reverse bridge,
`fable-r2.md:1117-1118`). G3 §10 adopts Opus's mechanism but pairs it
with L4's replace-to-zero end state — a combination nobody proposed:
it pays both for the internal C ABI surface that constrains the
evaluator during M3–M7 *and* for the full Rust re-port afterwards,
while deleting Opus's claimed saving. The doc should present §10 as
"Opus's mechanism under the lead's end state, contested by the other
three, decided by D0" (B6), not as the merged position. Related
attribution slip: the names.c registry is credited "(Muse)" — right
(`muse-r2.md:188-199`) — but Muse's generator emitted arity/eval/
visibility metadata with implementations in Rust vs base-*package*
natives; §10 extends it to link GNU `do_*` functions, which is Opus's
idea wearing Muse's name.

**X2. Pin-only finalizers were a 2–2 split on a safety argument; §3/§15.1
recast it as a measurement.** Fable + Opus take precise-decides/pins-pin;
Astra ("Never move conservative marking after weak classification …
a compatibility limitation to measure, not a proof of exact
finalization", `astra-r2.md:113`) and Muse (`muse-r2.md:70`) reject
it because a finalizer can free a native resource under a pointer C
still holds — a soundness claim no finalizer-*delay* count can
adjudicate. §3's "decided by experiment A" papers over the split; the
fix is B10's two-probe experiment plus recording both positions in
`docs/g3-council.md` as a named dispute with the lead's resolution.

**X3. Allocation semantics: two of four positions vanish, and Muse's
fallback is inverted.** §15 D1 lists "safepoint-only (default); Muse's
auto-rooting allocator (fallback)". In `muse-r2.md` auto-rooting is the
*primary* design (allocation collects; the alloc frame keeps fresh
results alive, `:403-427`) and its stated fallback, if it costs >2% on
`calls.R`, is *explicit `keep()` + branding* (`:429-433`) — not the
converse. Astra's position — allocation itself takes `&mut Cx`, roots
are scope slots, the brand "proves a read interval, not reachability"
(`astra-r2.md:16,80-85`) — appears nowhere in the decision table, not
even as a measured variant. D1 should list all three sound variants
(safepoint-only; collecting allocator + auto-root frames; `&mut`
allocation + scope roots) with the pre-registered rule, or record why
two were eliminated on grounds other than measurement.

**X4. Hashed environments "GNU R bit for bit" (§5) was Fable's position,
contested, and gets no decision row.** Muse took the opposite design
knowingly — own open-addressing table + order ranks, reverting to
Fable only on a single unexplained order diff (`muse-r2.md:169-187`) —
and Astra specified an own index with recorded enumeration order
(`astra-r2.md:150`). Namespace/global lookup is on every hot path, so
this is a fastest-goal question, not a detail. Add decision D2h (GNU
chained HASHTAB vs own table + order ranks; bars: namespace-lookup
instructions, `ls`/`as.list`/`serialize` byte identity). Note also the
tension nobody priced: "bit for bit" is incompatible with §8's 24-byte
header and per-type node sizes — at most the *logical* structure
(FRAME/HASHTAB shape and traversal order) can be bit-for-bit once the
header changes; say which.

**X5. Dropped load-bearing mechanisms (each was someone's safety or
exactness argument, none is mentioned as dropped):**
- Fable's type-enforced ESCAPED hand-outs (`FrameRef` converting only
  through `Cx::expose_env`, `fable-r2.md:748-753`). §5/§13 keep frame
  recycling and its zero-allocation bar but lose the mechanism that
  makes a missed hand-out a compile error instead of use-after-recycle.
- Astra's pending-exit stack (see B11) and stable-address context
  slabs with saved protection/root tops per record (`astra-r2.md:184`);
  §6's context paragraph has the chain but no saved-state list, which
  is where unwind bugs live.
- Muse's three-way differential gate: §9 downgrades the reference
  interpreter to "may exist only behind `ROBIGO_REF_EVAL=1`", while §16
  still lists it as a verification mode that "catches engine semantics"
  — if it is optional, the one-engine design has no independent oracle
  during M3, exactly when it needs one (`muse-r2.md:155-165` made it a
  permanent CI gate).
- Astra's S3-cache caution: a cache keyed on the class STRSXP's
  identity is invalid because C can `SET_STRING_ELT` the same object
  (`astra-r2.md:44`, against Fable/Opus's identity cache). §9/§6 say
  nothing about S3 cache keys; the guard must key on contents or on a
  version the C store path bumps.
- Muse's retarget-first risk (B1, `muse-r2.md:114-124`): L3 decided
  the opposite, which is the lead's right — but G3 drops the mitigation
  too. Until the evaluator is complete, `robigo-rt` should *also* be
  scored against the 4.5.0 expected set, with the NEWS-attributed list
  as the only allowed deltas; that separates representation bugs from
  version drift while both oracles exist. §14.2's flip-only gate
  discovers confounds at the worst moment.

**X6. Small misquotations that will mislead an implementer.**
- §7: "cached in a spare word of its CHARSXP (Fable)". Fable proposed a
  u32 symbol id at `+12` — the spare four bytes beside the hash, and
  only under the 24-byte layout (`fable-r2.md:1308`). Today's CHARSXP
  has no spare word (`crates/robigo-core/src/lib.rs:163`).
- §2: "`View` keeps today's `Value` variant names so matches convert by
  renaming arms." Under plan B nothing converts; builtins are re-ported
  (or, per §10, compiled from C). Keep the names for familiarity, but
  the stated reason belongs to the rejected conversion plan.
- §3 item 4: "Pending finalizers run where GNU R runs them (eval-count
  poll, after `gc()`, after a native call returns)." GNU R also runs
  them inside `bcEval`'s poll (`R462/src/main/eval.c:6247`) and the
  eval-count threshold is 1000 *evaluations* (`eval.c:1093-1099`); in a
  compiled-only engine "eval count" needs redefining (VM ops? calls?),
  because finalizer *timing relative to printed output* is observable.

#### 4. Answers to §17's open questions

##### Q1. Can any of `rlib/` stop being vendored?

The *files* can; the *content* cannot. Distinguish three layers:

1. **Runtime vendoring — removable, and already half-removed.** GNU R's
   installed tree contains no base `.R` sources: it ships a baseloader
   plus `base.rdb`/`base.rdx`, and robigo already prefers exactly that
   path when an installed base exists
   (`crates/robigo-runtime/src/bootstrap.rs:25-35`); the embedded
   `rlib/` sources are the fallback (`ROBIGO_BASE=src`). G3 should make
   the rdb path the only path for `robigo-rt`: build the lazy-load
   database at build time (with the GNU R 4.6.2 oracle's
   `tools::makeLazyLoadDB`, or with `robigo-rt` once its serializer is
   byte-trusted), ship the `.rdb`, and delete the source-eval fallback.
   This is also *more* exact: srcrefs, bytecode (B5) and lazy data then
   match GNU R's installed artifacts by construction.
2. **Build-time vendoring — not removable, only relocatable.** The
   database's contents (bodies, formals, srcrefs, lazy data, `sysdata`)
   are observable data (§10's own list, plus `getParseData`,
   `getSrcFilename`, `deparse` output and namespace internals reachable
   via `getFromNamespace`). They must be derived from GNU R 4.6.2's
   sources byte-for-byte. Whether the repo checks in 166 files
   (1.2 MB today) or fetches a hash-pinned tarball at build time (the
   `scripts/build-natives.sh` pattern, SHA256-checked) is a
   reproducibility choice, not a semantics one. My recommendation:
   keep `rlib/` checked in (hermetic, diffable on version bumps) but
   treat it purely as build input; "designed away" then means no `.R`
   text in the shipped artifact and no source evaluation at startup.
3. **Replacement by Rust — only behind the existing objects.** Any base
   function can gain a Rust intrinsic fast path (§10's `match.arg`
   example) provided the closure object, its frame semantics under
   `sys.call`/`trace`/`Rprof`, and its error calls survive — i.e. the
   intrinsic is a guarded shortcut *inside* the call, disabled when
   traced/debugged/profiled. The profile sources (`Common.R`,
   `Rprofile.unix`) are in the same class once executed: their products
   (`.First.sys` and friends) are base objects in the database.

So the honest end state for the lead's preference: no GNU R C in the
binary (L4, achievable), no GNU R R *source files* at runtime
(achievable via rdb), but GNU R's base *as data* forever — because
"fully compatible" makes that data part of the spec.

##### Q2. A third option for C frames beyond pins vs strong roots

Yes: **pin for memory, defer finalizer *execution* to segment close,
keep weak-reference clearing precise.** Concretely: precise marks
decide readiness (as in pin-only); conservative words then pin memory;
but an object that is finalizer-ready *and* pinned-only has its
finalizer queued to the close of the innermost open C segment that
pinned it, instead of the next general safe point, while weak
references to it clear immediately (GNU R's timing, observable from R).
What this buys: correctly protected packages are unaffected in every
observable (anything they still hold is `PROTECT`ed, hence precisely
marked, hence never ready); incorrectly protected packages keep both
memory *and* native resource until their C frame is gone, which is the
safety strong roots were actually providing. What it costs: a finalizer
for an object referenced only by a *stale* word in a live C frame is
delayed until that `.Call` returns — bounded by segment duration,
unlike strong roots where the delay extends to the next collection
after the word happens to be overwritten. The stub's narrowing +
zeroing then directly shrinks the stale-word set, which prices the
stub's instruction cost against finalizer delay in experiment A.
Optional sharpening, with in-repo precedent: a per-DLL exact mode
(scan off entirely) for packages that opt in, mirroring core.md §10's
isolate opt-in (`docs/core.md:1004-1009`) — audited packages get GNU
R's exact semantics by declaration; everyone else gets the safe mode.
Either way, B10's premature-finalizer probe is the instrument that
tells the three apart; the delay-only stress cannot.

##### Q3. Which builtins can be designed away?

First, a bound the doc should state: **none can be deleted.** Every
`.Primitive`/`.Internal` is a first-class value — reachable via
`get("+")`, `.Primitive("+")`, `do.call`, passed to `lapply`, printed
by `args()`, dispatched through internal generics (`Ops`, `Math`,
`Summary`, `[`, …). "Designed away" can only mean: the VM/JIT own the
hot path, and the function that remains is the generic, dispatching,
fully-checking fallback. Under that definition:

*Design-away candidates* (hot path in VM/IR, fallback ported):
arithmetic and relational/logical operators (scalar and vector kernels,
with GNU R's warning sites); `:`/`seq_len`/`seq_along` (ALTREP
construction is one VM op); `[`, `[[`, `$`, `[<-`, `[[<-`, `$<-` in
their non-dispatching cases; `length`, `typeof`, `is.*` type tests,
`nrow`/`ncol`-style attribute reads; `c` for the no-dispatch,
same-type cases (generic `bind.c` semantics remain the fallback);
`sum`/`prod`/`min`/`max`/`any`/`all` reductions on unclassed vectors;
`rep` (ALTREP/wrapper cases); `match`/`%in%` pointer-hash fast path
(§7); `paste0`/`paste` with constant separators; `seq`/`seq.int`
common forms. The guard conditions are the design: binding still
resolves to base's function (search epoch), no class (or class handled
by the dispatch the intrinsic itself performs), ALTREP already
materialized or robigo-owned, and warnings/errors raised at GNU R's
sites. GNU R's own `cmp.R` inline list and today's
`docs/ledger/perf-vm.md` "For G3" list are the starting inventory.

*Must be ported as functions* (no meaningful intrinsic): everything
that introspects or mutates execution state — `eval`, `substitute`,
`quote`, `match.call`, `sys.*`, `parent.frame`, `assign`/`get`/
`exists`/`rm`, `on.exit`, condition machinery, `UseMethod`/
`NextMethod`, `delayedAssign`, `trace`/`debug` internals; everything
whose semantics *is* a large specified algorithm with dispatch —
generic `c`/`cbind`/`rbind` (`bind.c`), `order`/`sort` generic cases,
`match` on mixed encodings, coercions (`coerce.c`); and everything
bound to the outside world — connections and I/O, `serialize`/
`unserialize`, regex (`regex.c`/TRE), graphics device internals,
dynamic loading, `system`/`pipe`. The §10 sentence should be amended
to carry the first paragraph of this answer; as written ("keep only a
generic fallback") a reader could budget the fallback as trivial. It
is not: for `[` and `c` the fallback is most of the semantics.

##### Q4. What is missing from §12?

At least these rows (mechanism + case each, per §12's own rule):

- **BCODESXP and the compiler package** (B5): `disassemble` output,
  `identical(ignore.bytecode)`, `enableJIT` levels, JIT compile
  *timing*, packages' GNU-R-built lazy-load databases, bytecode in
  `saveRDS` both directions.
- **Debugging/profiling observables**: `trace` entry/exit prints,
  `debug`/`browser` stepping granularity in compiled code (step points
  in the IR), `traceback`/`.Traceback`, `dump.frames`/`recover` frames
  that outlive their contexts (forces materialization), `Rprof` and
  `Rprofmem` records, `options(error=)` expressions.
- **S4**: §8 reserves the bit but §12 has no row — `methods` dispatch
  and its cache invalidation on `setMethod`/`setClass`, `@`/`slot<-`
  validation, the non-API hooks `methods.so` needs.
- **Active bindings and lazy-load promises in hashed environments**:
  the 4.6 binding API must answer there too, from GNU-shaped binding
  cells — a second mechanism beside slots (B13).
- **Serialization beyond the one row**: reference-index order induced
  by slot listing order (B4); cross-runtime RDS (GNU-saved `1:n` loads
  as robigo compact ALTREP and vice versa; ALTREP class hooks);
  active-binding and environment serialization errors.
- **Private objects in reports**: Shape/Code/overflow objects must be
  invisible or correctly mapped in `memory.profile()` type counts,
  `object.size()` (including its environment non-recursion rules), and
  `gc()` Ncell accounting under per-type node sizes.
- **Growth identity**: §4's invisible in-place extension vs GNU R's
  copy is visible through `tracemem` and `.Internal(address())` on the
  grown binding *(to verify with an oracle script; GNU R grows in
  place only when GROWABLE with room, `R462/src/main/subassign.c:161-176`)*.
- **Partial-match and option-dependent warnings** under memoized
  matching (B13c); `warnPartialMatchAttr/Dollar` under intrinsic `$`.
- **Interrupts**: `R_CheckUserInterrupt` from long C loops and long
  Rust kernels (B3) is a non-catchable jump to top level, not an
  error; its timing against output is observable.
- **Fork**: `parallel::mclapply` forks the heap; core.md Decision A.1
  (side mark bitmaps, few dirty pages) constrains any new layout,
  value-stack reservation and JIT code — §8's bake-off metrics don't
  include fork dirty pages.
- **`identical`/`duplicate`/`all.equal` vs the 4th word**: code objects
  and weak cache words must be invisible to all three, and `duplicate`
  of a closure must not alias a mutable code object (B5).

##### Q5. Are §13's bars the right definition of "fastest"?

Directionally yes; as gates, two fixes and three missing classes.
Fixes: (i) §13 conflates instruments — rivals are wall-time only
("instructions are not time, and the fastest R is not GNU R",
`scripts/rivals.py:2,11-13`); instruction counts exist only against
GNU R via `perf.py`. Say so per row. (ii) "Fastest on every benchmark
in both modes" and "package suites below GNU R for every package" are
unattainable as stated for native-bound workloads (data.table/Rcpp
time inside the package's own C/Fortran has a floor at parity); use a
non-inferiority margin with a noise model (e.g. within 1% or
statistically indistinguishable at the suite level) for those, and
keep strict wins for evaluator-bound rows. Also state which build the
rival numbers use (PGO is named for wall time; the one-process mode
includes startup, so base-load time deserves its own named bar —
`library(dplyr)` end-to-end, not only "`-e 1` ≤ today").

Missing workload classes: **I/O and text** (`read.table`/`scan`/
connections, large `gsub`/regex, `sprintf` formatting — String
building is listed, parsing input is not); **S4-heavy code**
(Bioconductor-style dispatch; no benchmark names it, and G6 will need
the number); **long-running session behaviour** (RSS growth and p99
pause over a long mixed session — fragmentation and CHARSXP-cache
behaviour under churn — plus fork-worker throughput for `mclapply`).
Compile-time cost also needs a bar of its own under the one-engine
design: `source()` of a large file and `eval(parse(...))` throughput
(D11's real stakes), where today §13's "startup ≤ today" is the only,
undefined, proxy.

#### 5. Non-blocking improvements, ranked

1. **§15.3 E4 needs a defined instrument.** "Bytes allocated between
   consecutive safepoints that the next collection frees" requires the
   collector to attribute freed bytes to a birth epoch — a per-safepoint
   allocation counter with birth-epoch side data, or an approximation
   (`alloc_since_gc` already exists per collection,
   `crates/robigo-core/src/gc.rs:97,595`). Name the instrument and who
   builds it in M1, or E4 is unmeasurable on the day it gates.
2. **§9: sequence the two native backends.** Cranelift tiers *and* an
   LLVM tier *and* a feedback-directed tier is three deopt/exactness
   surfaces to keep bit-identical. Effort is free; verification surface
   is not. Gate the second backend on a measured Cranelift plateau
   (named benchmarks where its codegen is the binding constraint), and
   say the feedback tier lands before any second backend.
3. **§7: state the uncached-CHARSXP invariant.** Pointer hashing in
   `match`/`unique` is sound only while equal bytes+encoding imply
   identical CHARSXP. The D10 fallback (uncached intermediates) breaks
   that implication silently. One sentence: uncached CHARSXPs carry a
   header mark; any hash-based builtin that meets one hashes bytes for
   the whole vector. Add a conformance case (`paste`-built strings
   through `duplicated`).
4. **§6: `promargs` lazy materialization vs C-visible `RCNTXT`.** Base
   natives compile against robigo's `Defn.h` and can read context
   fields directly; a lazily materialized field inside a C-visible
   struct needs its sentinel discipline written down (what value sits
   in the field before materialization, which entry points force it).
   One paragraph now saves a redesign at M5.
5. **§14: split the flip from the deletion.** "One PR deletes
   `robigo-runtime`, the bridge, …" makes the highest-risk change
   unreviewable and hard to bisect. Flip the default in one PR, delete
   in a follow-up within days; L1's intent (nothing stays half-alive)
   survives, bisectability appears.
6. **§11: initial-exec TLS in installed headers is a `dlopen` budget.**
   *(guess, to measure)* Initial-exec TLS in many package `.so`s draws
   on glibc's static TLS surplus; loading very many DLLs can exhaust
   it. Add a load test (the full package corpus's DLLs in one session)
   to D7's evidence before committing the header macro to that model.
7. **§8: the census caveat is aimed at the wrong layer.** Packages are
   built against robigo's headers, so a 24-byte header is safe exactly
   when robigo's *installed headers* hide what GNU R 4.6.2 hides
   (`R462/doc/NEWS` hidden list) and its exports match; `symbols.tsv`
   rows for `SETLENGTH`/`SET_TRUELENGTH`/`SET_PRVALUE`
   (`tests/capi/symbols.tsv:210,213,252`) are 4.5-era package uses that
   will fail to build on GNU R 4.6.2 too. Reword the parenthetical to
   make header-parity the requirement and the census a check on it.
8. **§13/§14: define "startup ≤ today"** (which mode — installed
   `base.rdb` or source fallback — and which binary, §14's own
   `robigo-rt --vanilla -e 1`), and add the `library(<pkg>)` load-time
   bar from Q5.
9. **§10: C-count ratchet needs a denominator and a clock.** "May only
   go down" from what initial count, counted how (registry rows?
   `src/main` files? exported `do_*` symbols in the binary)? If B6's
   D0 adopts the crutch, publish the counting script with M3 so the
   ratchet is mechanical like the others.
10. **§2: keep `trybuild`, add the re-entrancy cases (B1) to it** —
    a `Ref` live across an ALTREP callback and a second `&mut Cx`
    minted inside a C export are the two compile-fail cases that
    actually protect the design; today's listed cases are the easy
    ones.

#### Closing

None of this argues against starting soon. The road — one heap, plan B
on main, safepoint collection with branded refs, GNU R's REFCNT and
binding semantics under 4.6.2, measurement-gated layout choices — is
the right road to the fastest fully compatible R, and §12/§15/§16's
instinct (name every observable, pre-register every bar, give every
scary mechanism an off-switch) is exactly how this project has earned
its ratchets so far. What the doc needs before M1 is one more pass of
the kind it demands of the code: every mechanism named, every merged
disagreement recorded as decided-by-whom, and no load-bearing claim
without its instrument. With B1–B9 answered in text, I would score it
8/10 and start M1.



## Round 4

### Brief

### Council round 4: re-review the revised G3 design

Same council, repository and read-only rules as before (`brief.md`). No
edits to the repo, no commits, no subagents, no builds into the repo.

`docs/g3.md` (worktree, uncommitted) is now v2. It was rewritten from all
four round-3 reviews (`astra-r3.md`, `fable-r3.md`, `opus-r3.md`,
`muse-r3.md` in this directory) and experiment A's results
(`expA-results.md`). The lead's goal still binds: the fastest R runtime,
fully compatible; effort is free.

Deliverable, in the file named in your task message:

1. **Score out of 10** (same question as round 3: does the doc get robigo
   to the goal, and is it specific enough to start M1?).
2. **Your round-3 blocking findings, one by one: fixed, partly fixed, or
   not fixed**, quoting the v2 text that addresses each, and for anything
   not fully fixed the smallest replacement text.
3. **New blocking findings only if v2 introduced them** (a contradiction or
   an unsound rule created by the rewrite). Do not reopen settled lead
   decisions (§1) or raise new scope; list anything else as non-blocking,
   at most five items.

Final chat reply: the file path, the score, and one line per finding still
open.


### Astra

### Astra: council round 4

**Score: 9/10.** V2 preserves the performance goal and makes the implementation contracts much more concrete. Four of my seven round-3 blockers are fixed. Three are partly fixed and need small edits before M1's safe API and verification rules are adopted. M1 implementation can start alongside those edits; I would not freeze its contract with the remaining exceptions intact.

This is a read-only review of [g3.md](<repo>/docs/g3.md), the round-4 brief, my round-3 review and experiment A's results. I checked the RNG counterexample against the supplied GNU R 4.6.2 source. I ran no builds or experiments and changed no repository files. Section and line references below refer to this version of g3.md. I accept §1's lead decisions and do not reopen them.

#### Round-3 blocking findings

##### B1. Generic ALTREP length — fixed

§2.4, lines 99–103:

> **Length, data pointer and element access on any other ALTREP take `&mut Cx`** on a root

> `Ref::len()` returns `Option<usize>` (`None` for a package class); `cx.xlength(root)` calls the method.

This removes the unsafe generic token-free query. The own-class restriction to allocation-free, non-signalling reads and §2.2's general callback rule supply the required boundary. No replacement needed.

##### B2. Root ownership and re-entry — partly fixed

§2.2, lines 57–63:

> `Cx` is a `!Send + !Sync` capability over `*mut Isolate`; the isolate's fields live in `UnsafeCell`s behind audited accessors.

> A C callback into an export obtains its capability from the isolate pointer while the outer one is suspended

§2.3, lines 84–90:

> `Root<'s>` is an index into that stack branded by a generative scope

> A root is rebound only by its owning isolate (checked owner identity in debug builds; negative tests). Output is written into the caller's `Out` slot before callee roots are released.

Stable slots, scope lifetime, output transfer and suspended re-entry now have explicit mechanisms. The revised distinction between read-interval safety and rooting is also correct.

**Still open:** debug-only identity checks do not enforce a safe API in release builds. The doc expressly says the borrow checker does not prove same-isolate use. A scope brand does not by itself prevent passing its root to an unrelated `Cx`; minting and reading need ownership enforcement as well as rebinding. Otherwise safe code can read another isolate's slot index, root an object in the wrong heap, or keep a view while the actual owner collects. Root clearing/rebinding should also explicitly end existing read leases.

**Smallest replacement** for the owner-identity sentence, plus its immediate contract:

> Every safe operation that reads, mints from, publishes or rebinds a handle enforces matching isolate ownership, by an invariant isolate brand or an owner check enabled in every build. Debug-only checks are sufficient only where another enforced invariant already proves ownership. Clearing or replacing a root requires the owning `&mut Cx` and ends outstanding read leases. Root scope storage remains live independently of a temporary execution-capability borrow. M1 tests cross-isolate operations in release mode, root reset with a live view, scope escape and nested native re-entry; Miri covers the non-FFI paths.

##### B3. Nested pending exits — fixed

§6.5, lines 359–369:

> **pending-exit stack** in the isolate, a precise root

> parks the record in the context being left [...] and restores it afterwards, unless that code leaves by its own exit beyond it

> Each retained unwind continuation owns its record until resumed or released

> Sizes are asserted for the hot-path result types, not promised for every `T`.

This supplies the missing ownership and save/restore protocol without giving up the small error token. E8 adds both caught-cleanup-error cases. No replacement needed.

##### B4. Mutation validity of compiled code — fixed

§9.3, lines 451–458:

> Code, match plans and dispatch caches validate every mutable dependency: BODY and FORMALS spines, class vectors, bindings, numeric policy. Pointer identity alone is never a validity guard.

> foreign effects that cannot be tracked (raw C writes) conservatively invalidate assumptions at native entry, at callbacks and on return.

> A suspended compiled activation whose assumption broke deopts at its next uncommitted operation.

Together with §9.2's prohibition on effect replay, this covers the missing correctness contract. §6.1 explicitly defers to this fuller rule, so its shorter top-level invalidation description is not a contradiction. No replacement needed. Specific regression cases remain a non-blocking implementation reminder below.

##### B5. Differential execution and forced-copy gates — partly fixed

§10, lines 496–503:

> Everything else, and anything unclassified, is compared out of process

> results, post-state, output, warnings, callback order and external effects (fixtures for non-replayable inputs) must be identical.

E5 now says:

> trace counts, `inspect` fields and allocation diagnostics are excluded there and checked against the oracle in production mode.

Those changes resolve most of the original finding. Two exceptions remain.

**In-process RNG replay:** §10 still allows `rng` builtins to run twice with generator state saved and restored. An RNG operation can run arbitrary code, not just update generator state. In [RNG.c:330](<R-4.6.2 sources>/src/main/RNG.c:330), `GetSeedsFromVar` evaluates a promised `.Random.seed`. At line 143, `USER_UNIF` calls `User_unif_fun`. Restoring generator state cannot undo effects from either. A static builtin classification also cannot make a package ALTREP callback pure.

**Forced-copy identity:** E5 still promises identical values and effects on “every prototype test.” Its exclusions omit deliberate native alias mutation. An extension that writes through one alias can expose whether the debug mode inserted a copy through another. This was an explicit part of B5 in round 3, and E6 includes native cases in M1.

**Smallest replacement** for §10's in-process exceptions:

> Same-process dual execution is restricted to operations proven effect-free for the actual inputs and execution path, including argument duplication. Callback-capable inputs or paths, RNG operations and unclassified cases use independent processes. Capturing warnings or restoring RNG state is not an effect rollback. Compare final RNG state as well as the other listed post-state.

**Smallest replacement** for E5's first clause:

> Under `ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1`, compare semantic values and effects on prototype cases where the debug transformation is unobservable. Deliberate native alias-mutation cases have separate production-mode oracle expectations, alongside the trace, inspect and allocation-diagnostic exclusions below. Neither control weakens production compatibility.

##### B6. Allocation failure and collection observations — partly fixed

§3 item 3 now says:

> tests `due(size)` for the prospective request, the configured limits, and collects first if needed

§3 item 5 now says:

> returns an unsignalled `Err(NeedGc)`; nothing runs under `&Cx`.

> retries only the failed allocation through `&mut Cx` [...] no committed effect is replayed.

The collection-observables paragraph explicitly requires policy revision if exact observations cannot be kept. E9 tests low limits and failure after an effect. §0 also resolves the variable-address/time qualification. These fix the substantive omissions from round 3.

**Still open:** item 5 then adds:

> Builtins the registry marks restartable (no effect before their first allocation) may simply be re-run.

That condition is too weak for failures after the first allocation. A builtin can allocate successfully, commit a visible store or callback, then fail its second allocation. It satisfies the stated condition, yet rerunning it repeats the effect. This exception contradicts the preceding no-replay rule and E9. It is the residual part of the original allocation-retry finding, not a request for another mechanism.

**Smallest replacement** for that final sentence:

> A whole builtin may be re-run only if no committed observable effect has occurred before the particular failed allocation. Otherwise resume at that allocation with rooted state. Having no effect before the first allocation does not establish restartability after later allocations.

##### B7. What experiment A proves — fixed

§15.1 now explicitly preserves the objection:

> a pinned external pointer's finalizer can release a resource an unprotected C local still uses.

It then bounds the conclusion:

> The experiment shows compatibility on the tested cases; it does not prove resource safety for unprotected C, which GNU R does not give either.

The document also keeps native-enabled package and corpus runs pending rather than presenting the locally native-disabled conformance run as sufficient evidence. The export-rooting requirement addresses a distinct, necessary protection boundary.

This resolves my blocking objection: the design no longer promises resource safety from storage pinning. I accept pins as the selected policy under that stated limit. I do not require strong conservative roots or a reversal of experiment A. An adversarial resource-release test and a label for the per-DLL fallback remain useful follow-ups, not grounds to block this decision.

#### New blocking findings introduced by v2

None beyond the residual exceptions identified under B2, B5 and B6. They remain within the round-3 scope. I am not adding architectural requirements or reopening settled lead decisions.

#### Non-blocking follow-ups

1. Add named M1 regression cases for B1 and B4: a Length callback that allocates and re-enters R, in-place body/formals-child mutation, and mutation during a callback while compiled code is suspended. The contracts are fixed; the tests should lock them in.
2. State that the per-DLL strong-root fallback can delay finalizers and is a separate compatibility policy. A pin report alone cannot identify a missing `PROTECT`: a stale word and a live unprotected pointer are indistinguishable to the scanner. Describe the report as a candidate for investigation. Keep the external-buffer finalizer probe to demonstrate the documented resource-safety limit.
3. Distinguish debug torture collection at each robigo safepoint from the oracle contract for R-level `gctorture(TRUE)`. §3's requirement to revise the policy on observable differences already governs this; “collects at every safepoint” should not by itself be read as proof of equivalence to GNU R's allocation-triggered torture.

The remaining changes are narrow contract edits. The proposed heap, execution model, migration and performance programme can proceed without another architectural redesign.


### Fable

### G3 round 4: re-review of `docs/g3.md` v2 (Claude Fable 5.2)

Base: main `4cff0ceb` plus the uncommitted `docs/g3.md` (v2, 783 lines).
Read-only; nothing was built or run. `R462/` is
`.../scratchpad/r462/R-4.6.2/src`. `g3.md:N` is a line of v2. Every source
line cited below was read in this round. Claims I did not check against a
running GNU R are marked "(not run)".

#### 1. Score

**9 / 10** (round 3: 7).

- All ten of my round-3 blocking findings are answered in the text. Nine
  are fixed outright. B3 is fixed as a rule and as an argument; the
  evidence it still owes is listed but has no milestone.
- The rewrite created three small errors (section 3). Two touch code M1
  writes (closure state, binding kinds); each is a one-paragraph or
  one-row correction. The third blocks D8 (M2), not M1.
- M1 can start once N1 and N2 are merged. Nothing else in the doc stops
  it.

#### 2. Round-3 blocking findings

| # | finding | status |
|---|---|---|
| B1 | one pending exit is not enough | fixed |
| B2 | caller-forced argument reported as forced | fixed (the new table has one wrong row: N2) |
| B3 | pins: argument, export rooting rule, evidence | rule and argument fixed; evidence unscheduled |
| B4 | E4's metric; `LARGE`; allocation failure | fixed |
| B5 | 24-byte header against `TRUELENGTH` in `src/main` | fixed (one consequence: N3) |
| B6 | slot frames: unbound state, definition order | fixed |
| B7 | in-place extension is visible | fixed |
| B8 | which profile runs when | fixed (one consequence: N1) |
| B9 | one `robigo-core` for two runtimes | fixed |
| B10 | `ROBIGO_BUILTIN_DIFF` on effectful builtins | fixed |

##### B1. Fixed

`g3.md:359-369`: "the exit (kind, target context, mask, value, condition,
call) is the top record of a **pending-exit stack** in the isolate, a
precise root. Code that runs during an exit (`R_run_onexits`, a `cend`, an
`R_UnwindProtect` cleanup, a calling handler, a finalizer) parks the record
in the context being left ... and restores it afterwards, unless that code
leaves by its own exit beyond it, which supersedes it as in GNU R."

- The cited lines hold: `context.c:198-209` (`first_jump_target` writes
  `jumptarget`/`jumpmask`), `:298-319` (the saved `R_ReturnedValue` around
  `on.exit`), `:977-993` (the unwind continuation takes them over).
- The stack is in the root list (`g3.md:166`) and in §11
  (`g3.md:524`). E8 (`g3.md:739-742`) is the test I asked for, plus the
  `R_UnwindProtect` case.

##### B2. Fixed

`g3.md:298-307`: "**A slot's kind is the kind GNU R's binding would have at
the same moment, never the cheapest one.** ... An argument the caller
evaluated early is still a **delayed** slot with the value attached as a
precomputed result ... A constant argument is a delayed slot under the
`Eval` profile (`eval.c:promiseArgs`, the final `mkPROMISE`) and a value
slot under `Bc` (`PUSHCONSTARG`, `eval.c:8042-8050`)."

- The §12 row is there (`g3.md:548`). §6.3's table is more than I asked
  for and is the right shape. One row of it is wrong (N2).
- Citation nit: `promiseUnwrap` is the function above `BINDING_TYPE`
  (`envir.c:780-805`); `envir.c:840-856` is `SYMBOL_BINDING_TYPE`.

##### B3. Rule and argument fixed; evidence unscheduled

- Rule: `g3.md:152-157`, "C API exports that may collect ... **root their
  `SEXP` arguments precisely for their duration** (GNU R's API functions
  protect their arguments; robigo's precise roots must contain GNU R's)".
- Mark order: `g3.md:170-174`, "no conservative word decides a weak
  reference or a finalizer".
- Argument and the 2-2 split: `g3.md:672-682`. This is the answer I gave,
  stated fairly, with Astra's and Muse's objection named.
- Evidence: `g3.md:682-687` lists what is "still to run before the heap
  adopts it" (capi corpus and package suites with natives under gctorture,
  the premature-finalization probe, narrowing and zeroing as separate
  switches). No milestone owns these runs. M1 already builds this mark
  order ("safepoint collection", `g3.md:618`), and `robigo-rt` cannot run
  a package suite before M5, so the runs can only happen on today's main
  with the experiment switches.

**Smallest fix.** Add to M0's "what" (`g3.md:617`): "experiment A's
remaining runs (§15.1) on main with the two switches". Add to M0's exit:
"those runs pass, or §3 step 3 is revised before M1 builds it."

##### B4. Fixed

- `g3.md:134-139`: `LARGE`, `cx.alloc_rooted(&mut self, size)`, which
  "tests `due(size)` for the prospective request, the configured limits,
  and collects first if needed"; the counted `&Cx` request above `LARGE`
  fails CI. `memory.c:2709-2714` does collect before it allocates.
- `g3.md:145-151`: failure returns `Err(NeedGc)`, "the caller ... retries
  only the failed allocation through `&mut Cx` ...; no committed effect is
  replayed." This is better than my text, which re-ran the builtin.
- E4 (`g3.md:724-727`) is my replacement; E9 (`g3.md:743-745`) tests the
  failure rule, including the effect that must not repeat.

##### B5. Fixed

- `g3.md:399-403`: "**Every variant keeps storage for what `TRUELENGTH`
  holds**: the exact growable capacity, the CHARSXP scratch
  (`radixsort.c`), hash-table fill counts (`HASHPRI`, `serialize.c`,
  `saveload.c`), with `inspect`'s `tl=` reproduced."
- `g3.md:488-491`: robigo's `Defn.h` maps `TRUELENGTH` onto that storage,
  and `scripts/check-cbase.sh` fails on unlisted uses.
- §5.2 keeps hash tables bit for bit, so `HASHPRI` has its word. D8's
  variant is now "24 bytes with §8's storage" (`g3.md:707`).
- The fix has one consequence with the shared symbol arena: N3.

##### B6. Fixed

- Unbound: `g3.md:235-238`, "**unbound** (reserved by the shape, never
  defined, or removed by `rm`: lookup continues in the enclosure;
  `exists`, `ls`, `R_envSymbols` do not see it)".
- Order: `g3.md:247-252`, "Each binding carries a definition stamp set
  when an unbound slot becomes bound (never on reassignment); enumeration
  lists by stamp descending; formals are stamped in reverse formal order
  at call entry. `rm` then reinsertion moves a binding to the front".
- I checked the rule against 4.6.2: `defineVar` prepends
  (`envir.c:1998`), reassignment keeps the cell (`:1988-1992`),
  `RemoveFromList` unlinks it (`:941-954`). Stamps descending reproduce
  that list in every case I could construct.
- The §12 row names the cases (`g3.md:546`). Where the stamp and the two
  new kinds are stored is not said (section 4, item 2).

##### B7. Fixed

`g3.md:210-215`: "**Growth**: `EnlargeVector` as written ...: GNU R's
expansion factor, and the same object exactly when GNU R keeps it
(unshared, growable, `truelength >= newlen`). Otherwise a new, untraced
object, as GNU R makes." Page remapping is D12, a measured variant. §12's
"identity is exact" row is replaced (`g3.md:542`).

##### B8. Fixed

- `g3.md:417-423`: "**Which profile runs is GNU R's decision**, tracked
  per closure in the code object: a port of `eval.c:R_CheckJIT`,
  `JIT_score`, `MAYBEJIT`/`NOJIT`, `R_JIT_STRATEGY` and the JIT cache ...
  `Eval` until GNU R's JIT would compile the closure, `Bc` after; `Bc` from
  the first call for installed byte code; `Eval` while the closure is
  debugged".
- The BCODESXP on observation (`g3.md:424-427`), BODY as a BCODESXP
  (`g3.md:286-289`) and the §12 row (`g3.md:551`) cover the rest.
- Putting the state "in the code object", which was my wording, now
  collides with §6.1's invalidation rule: N1.

##### B9. Fixed

`g3.md:605-611`: "`crates/robigo-heap` is a copy of `robigo-core` taken
when M1 starts; every heap change lands there. ... `robigo-core`,
`robigo-capi` and `robigo-runtime` are frozen (L2) and deleted at the flip
... No cfg-gated second layout exists in any crate." L1 and the appendix
agree. `robigo-rtapi` as a second fork is the consistent extension.

##### B10. Fixed

`g3.md:496-503`: effect classes; `pure` and `rng` in process; "Everything
else, and anything unclassified, is compared out of process ... results,
post-state, output, warnings, callback order and external effects". The
"anything unclassified" default is stricter than my text and right.

#### 3. New blocking findings (created by the rewrite)

##### N1. §6.1 against §9.1: invalidation drops GNU R's JIT state (`g3.md:284-294, 417-420`)

**Problem.** §9.1 keeps GNU R's JIT state "per closure in the code
object". §6.1 says "any write to BODY, FORMALS or CLOENV ... drops the
code object ...; `duplicate` of a closure drops the code object". Read
together, a copied closure loses its JIT state and runs `Eval` again. GNU
R's copy stays compiled.

```r
f <- function(x) x + 1
f(1); f(1)                      # GNU R's JIT compiles f
g <- f; attr(g, "k") <- 1       # duplicates the closure
`+` <- function(a, b) "mine"
g(1)                            # GNU R: 2 (not run)
```

**Evidence.**
- `R462/main/duplicate.c:302-312`: a CLOSXP copy takes the same BODY (the
  BCODESXP, if compiled) and "`if (NOJIT(s)) SET_NOJIT(t); if (MAYBEJIT(s))
  SET_MAYBEJIT(t);`".
- `R462/main/builtin.c:322-325`: R-level `environment<-` does the
  opposite for compiled code only, "switch to interpreted version if
  compiled", and leaves the two bits.
- C `SET_CLOENV` changes neither (`R462/main/memory.c:4632`, a plain
  setter).
- Today's runtime has this right: `JitState::clone` keeps the state and
  clears only the debug and trace bits
  (`crates/robigo-runtime/src/vm/rjit.rs:104-108`).
- `print(g)` also loses its `<bytecode>` line, and the BCODESXP made on
  observation is no longer the same object for `f` and `g`.

**Fix (replace the bullet at `g3.md:292-294`).**

> - Invalidation: any write to BODY, FORMALS or CLOENV, R-level or C, and
>   `duplicate` of a closure, drop robigo's compiled variants and the frame
>   shape. **GNU R's JIT state is not dropped with them**; it changes only
>   where GNU R changes it. Whether a body is compiled, its frozen
>   bindings and its BCODESXP belong to the body, so a `duplicate`d
>   closure shares them; `NOJIT` and `MAYBEJIT` belong to the closure and
>   are copied (`duplicate.c:302-312`). R-level `environment<-` returns a
>   compiled closure to `Eval` and keeps the two bits
>   (`builtin.c:322-325`); C `SET_CLOENV` changes neither. §9.3 has the
>   full mutation contract.

In §9.1 (`g3.md:417-418`) change "tracked per closure in the code object"
to "tracked per closure and per body (§6.1)".

##### N2. §6.3: the missing-default row gives the wrong binding kind (`g3.md:326`)

**Problem.** The row "missing / missing-default" answers a C binding query
with "missing" for both. For a formal with a default that the caller did
not supply, GNU R answers **delayed**, then **forced** after the first
read.

**Evidence.**
- `R462/main/eval.c:2272-2273`: the default is bound as
  `SETCAR(a, mkPROMISE(CAR(f), newrho)); SET_MISSING(a, 2);`.
- `R462/main/envir.c:807-830` (`BINDING_TYPE`): "missing" is returned only
  for `value == R_MissingArg`; a promise is delayed or forced. The
  `MISSING` bits are not read.
- So `f <- function(b = 2) .Internal(getBindingType(quote(b), environment()))`
  differs between the table and GNU R (not run).

This is the rule §6.2 states in bold, broken by the table that
implements it.

**Fix (replace the row at `g3.md:326` with two).**

> | missing (no default) | error per `R_isMissing` | → value, MISSING cleared | → unbound | missing | (none) |
> | missing-default (a delayed slot whose expression is the default, environment the callee frame, MISSING = 2) | evaluate the default; → forced, MISSING stays 2 | → value, MISSING cleared | → unbound | delayed, then forced | PROMSXP, as GNU R binds one (`eval.c:2272`) |

Add to E5 (`g3.md:728-731`): "`getBindingType` matches the oracle before
and after the first read for a constant, a symbol, a call and a defaulted
argument, on a closure's first call and after GNU R's JIT threshold."
`ROBIGO_MATERIALIZE=1` compares robigo with itself and cannot catch a
table that is wrong in both modes.

##### N3. §8 against §5.3 and L6: a `TRUELENGTH` scratch word on objects every isolate shares (`g3.md:384, 400-403, 269-274`). Blocks D8 (M2), not M1.

**Problem.** §8 now keeps the CHARSXP scratch that `radixsort.c` writes
(my B5). §5.3 and L6 put symbols in a process-shared arena, and
`docs/core.md:992-997` puts `R_NaString`, `R_BlankString` and the symbols
in "a shared, immortal, read-only segment". A symbol's PRINTNAME is the
cached CHARSXP of its name, so it is shared too. `order(c("", "x"))`
writes the scratch of `R_BlankString` and of the printname of `x`
(`radixsort.c:33-34`). Two isolates sorting at once race on those words,
or fault if the segment is mapped read-only. A package that uses
`SET_TRUELENGTH` on strings (data.table's `savetl`) does the same.

Neither rule was wrong alone; v2 states both and does not join them.

**Fix (add to the layout bullet, after `g3.md:403`).**

> A CHARSXP in the shared segment (`R_BlankString`, `NA_STRING`, every
> symbol's PRINTNAME) is immutable: its `TRUELENGTH` scratch, and any
> other word C may write, is a per-isolate word in a table indexed by the
> object's global id; `TRUELENGTH`/`SET_TRUELENGTH` branch on the shared
> bit. D8's bar includes this branch.

The same sentence covers §7's "spare CHARSXP word" for the cached symbol
(`g3.md:383-384`): on a shared CHARSXP it is written once, at interning.

#### 4. Non-blocking, at most five

1. **§3's collection-observables contract cannot be met as written**
   (`g3.md:182-187`). It asks for the `gc()` table, `gcinfo(TRUE)` reports
   and `Rprofmem` "oracle-tested", and says "if an exact observation
   cannot be kept under safepoint collection, the policy is revised".
   Cell counts, the number and level of collections and `Rprofmem`'s page
   lines depend on GNU R's object model (a cons cell per binding, a
   PROMSXP per argument, three generations), which §5, §6 and §8 replace.
   No collection policy keeps them. Split the list. Exact: finalizer
   delivery and order, what `gc()` and `gctorture(TRUE)` make happen,
   `mem.maxVSize()`, the limit errors, `Rprofmem` lines for vectors. Not
   compared, like addresses: cell counts, trigger sizes, collection
   counts and levels. Otherwise the clause that revokes D1 has no test.
2. **§5 calls a word map normative that its own amendments do not fit**
   (`g3.md:223-226`). `fable-r2.md` §3.1 gives a 2-bit kind with four
   kinds, all six low key bits used and all sixteen high bits used. v2
   adds two kinds (unbound, cell) and a stamp per binding. Say the map is
   indicative and M1 fixes it, with the constraints: the plain-value read
   stays one test on the key, unbound and cell go out of line, and the
   stamp lives outside the key (a `u32` per slot, or written only when
   the order departs from the shape's, as round 3 proposed). Also,
   `docs/g3-council.md` does not exist yet; a requirement should not live
   only in a transcript.
3. **M1 and M2 overlap, and M1's exit needs M2's work.** M1 builds
   poll-only allocation, the new roots and trace arms, and E3 needs
   `ROBIGO_GC_MOVE=1`; all are appendix rows that the table gives to M2
   (`g3.md:618-619`). Reword M2 to "the remaining heap changes: layout
   (D8, D13), removals" and mark the allocation, roots, trace and debug
   rows as M1.
4. **Three round-3 items are unchanged.** "After a native call returns"
   (`g3.md:159`) is still not one of `R_RunPendingFinalizers`'s four call
   sites (`eval.c:1096, 6247`; `memory.c:3093, 3101`). The root list
   (`g3.md:164-167`) still lacks the `R_alloc` stack and the list of
   objects awaiting finalization, and nothing derives it
   (`#[derive(Trace)]` on isolate state). D1 (`g3.md:699`) still omits
   Astra's `&mut Cx` allocation into an `Out` slot, or the reason it is
   out.
5. **Two bars have no number or type.** D0 adopts the crutch "only if the
   boot set is large" (`g3.md:484`); §15.2 says bars are written before
   the runs, so write the count. Two of its three pilot files
   (`arithmetic.c`, `subset.c`) are ones §10 says never come from C; add
   one the crutch would serve and that reads promises (`bind.c`). And
   `Err(NeedGc)` (`g3.md:146`) cannot be the zero-sized `RResult` token,
   whose kind lives in the pending-exit stack that `&Cx` code must not
   write: say allocation returns its own `Result<_, NeedGc>`, converted at
   the first `&mut Cx` frame.

#### 5. Findings still open, one line each

- B3: experiment A's remaining runs have no milestone; add them to M0.
- N1: `duplicate` and invalidation must not drop GNU R's JIT state
  (`duplicate.c:302-312`, `builtin.c:322-325`).
- N2: §6.3's missing-default row should answer delayed, then forced
  (`eval.c:2272`, `envir.c:807-830`).
- N3: the `TRUELENGTH` scratch of a shared CHARSXP must be per isolate
  (blocks D8, not M1).


### Opus

### Council round 4: re-review of `docs/g3.md` v2 (Claude Opus 5.5)

Base: worktree at `4cff0ceb` plus the uncommitted `docs/g3.md` v2 (783
lines). I ran nothing that writes into the repo. Line numbers are v2's.
`R450/` is `~/.cache/robigo-research/rsrc/R-4.5.0`. I checked the 4.6.2
citations against the round-3 reviews and the 4.5.0 sources where the
code is unchanged.

#### 1. Score: 8.5/10 (round 3: 7)

v2 fixes all nine of my round-3 blocking findings, most of them with my
replacement text or something stronger. Some fixes add detail I had not
asked for, and all of it is right: the cell kind stays for the binding's
lifetime because 4.6.2's `R_varloc_t` is `{ SEXP cell }`; the
binding-state transition table in §6.3; installed BCODESXP kept as the
real `BODY`; the export rooting rule in §3 item 6. The end state is now
consistent with L1–L4 and L6, and the crate graph matches L2.

It loses 1.5 points for four small defects the rewrite introduced or left
behind. Each is a one- or two-sentence text change, and none changes the
design:

- **N1**: §2.4 says robigo's own ALTREP methods are "allocation-free".
  That is false for one of the classes §8 lists, deferred coercions.
- **N2**: the per-DLL strong-root policy contradicts §0 and §17's own
  sentence.
- **N3**: base-package natives "replaced where measured faster"
  contradicts L4's "the end state contains no GNU R C".
- **B9 residue**: M1's exit needs parts of the appendix that M2 owns, and
  it needs `dyn.load` and `print`, which M5 and later own.

**Can M1 start?** Yes, once the B9 residue and N1 are fixed. Both touch
M1's scope or its API contract. N2 and N3 can be fixed in the same pull
request but do not gate M1.

#### 2. Round-3 blocking findings

##### B1. Crate graph and M-order vs L2: **fixed**

§14.1 (605-611): "`robigo-rt` has its own heap and C API crates from M1.
`crates/robigo-heap` is a copy of `robigo-core` taken when M1 starts;
every heap change lands there. `crates/robigo-rtapi` holds the C API
exports over `robigo-rt`, with no host table. `robigo-core`,
`robigo-capi` and `robigo-runtime` are frozen (L2)". M3 (620): "every
export the base boot needs in `robigo-rtapi`". M5 (622): "package loading
over `robigo-rtapi` (`dyn.load`, `R_init_*`, registered routines,
methods.so hooks)". §15.1 (687-688) also moves the stub removal into
`robigo-heap`, so the frozen core is not touched.

There is one residue in how the appendix is split between M1 and M2. I
list it under B9 because it is about M1's scope.

##### B2. The C crutch vs robigo's layouts: **fixed**

- §10 (476-478): "The evaluator-owned files (`eval.c`, `envir.c`,
  `errors.c`, `context.c`, `debug.c`, `memory.c`, `objects.c`, `match.c`,
  `main.c`: about 176 `names.c` entries) are Rust from M3."
- §10 (487-491): "robigo's `Defn.h` defines `FRAME`/`HASHTAB` (read-only
  snapshot of slot frames), `TRUELENGTH` (onto §8's storage) and `RCNTXT`;
  `scripts/check-cbase.sh` fails on any use of `FRAME(`, `HASHTAB(`,
  `TRUELENGTH`, `->promargs`, `->cloenv`, `->callfun` not in its
  allowlist."
- §8 (401-404): "**Every variant keeps storage for what `TRUELENGTH`
  holds**: the exact growable capacity, the CHARSXP scratch
  (`radixsort.c`), hash-table fill counts".
- §7 (383-384): the symbol cache "is not the `TRUELENGTH` scratch
  `radixsort.c:savetl` overwrites".
- §6.5 (349-351): "`promargs` is filled when a closure context is pushed
  if any compiled C reads it as a field (`util.c:594`, `do_nargs`)".
- §5.1 (256-257): `FRAME()` of a slot frame is a snapshot and
  `SET_FRAME` is an error.

D0 now gates the whole crutch, which is stronger than my fix. One
non-blocking gap is in its rule (§4 item 2).

##### B3. Unwinding through C contexts: **fixed**

§6.5 (345-358): "**One context chain: GNU R's linked `RCNTXT`** ... A
trampoline's jump record is a field of its `CTXT_CCODE` record; there is
no second shim stack." And: "Every exit, a Rust `Err` reaching an export
wrapper or a C `Rf_error`, runs `R_run_onexits` semantics ... **before**
any `longjmp`: the `cend` and `on.exit` of every record above the target,
innermost first, with `R_GlobalContext` set as GNU R sets it, from the
frame about to jump. A Rust `Err` that pops a C-pushed record runs the
same walk."

E8 (741-742) tests it ("a C context's `cend` runs before the jump; all in
every tier and across a C frame"), and §12 (556) has the row. The
pending-exit stack (359-369) also covers the nested-unwind case that I had
not raised.

##### B4. Slot frames: overflow, definition order, cell kind: **fixed**

§5.1 (230-252):

- overflow: "an **overflow area** (a slots object at a fixed offset, null
  until first use) for bindings not in the shape";
- unbound: "**unbound** (... removed by `rm`: lookup continues in the
  enclosure; `exists`, `ls`, `R_envSymbols` do not see it)";
- cell: "**cell** (... the binding stays cell-backed for its lifetime,
  because 4.6.2's `R_varloc_t` is `{ SEXP cell }`)";
- order: "Each binding carries a definition stamp set when an unbound slot
  becomes bound (never on reassignment); enumeration lists by stamp
  descending; formals are stamped in reverse formal order at call entry.
  `rm` then reinsertion moves a binding to the front, as in GNU R."

§12 (546) has the row, D2s (701) keeps shape transitions as a measured
variant, and §6.3's `cell` row ("`rm`: detach the cell") matches GNU R:
a holder of the old cell then writes to a detached cons.

##### B5. GNU R's JIT decisions and BCODESXP observables: **fixed**

§9.1 (417-427): "**Which profile runs is GNU R's decision**, tracked per
closure in the code object: a port of `eval.c:R_CheckJIT`, `JIT_score`,
`MAYBEJIT`/`NOJIT`, `R_JIT_STRATEGY` and the JIT cache ... `Eval` while
the closure is debugged ... A GNU R-shaped BCODESXP is produced on
observation ... by running GNU R's compiler package on the closure's
source; the `bytecode-fidelity` fixtures carry over."

§6.1 (286-289) adds that installed byte code is the real `BODY`, and §10
(513-516) builds the databases with GNU R's own `cmp.R`, so base closures
hold true BCODESXPs. §12 (551) has the row.

There is one residue, which my own round-3 text caused. It is listed as
non-blocking because it affects only closures whose environment changed
between compilation and observation (§4 item 1).

##### B6. Package ALTREP reads: **fixed** (the rewrite added N1)

§2.4 (97-104): "**Length, data pointer and element access on any other
ALTREP take `&mut Cx`** on a root (`R462/main/altrep.c:340`
`ALTREP_LENGTH` dispatches to the class's C method). `Ref::len()` returns
`Option<usize>` (`None` for a package class); `cx.xlength(root)` calls
the method. `Args` records whether any argument is a package ALTREP
object, so builtins take one branch."

This is my replacement text, extended to the data pointer and element
access. The new preceding sentence about robigo's own classes is wrong
(N1).

##### B7. D6 offered an inexact variant: **fixed**

§4 (204-205): "No eager variant is a candidate (D6 is a control only)."
D6 row (706): "deferred (the design); eager (control only, never
adoptable)". §0 (31-32) makes this a general rule: "An inexact variant is
never a candidate, only a control."

##### B8. Experiment A: result, meaning of pin, decision, split: **fixed** (the rewrite added N2)

§3 step 3 (170-173): "each word that points into an allocated object
marks it and traces from it, so its memory and everything it reaches stay
allocated; no conservative word decides a weak reference or a finalizer."

§15.1 (651-688) records the results. It gives the decision ("**Decision:
pins** (§3 step 3), with the export rooting rule (§3 item 6)"), records
the 2-2 split with Astra's and Muse's argument, and answers it: "GNU R
decides finalizers on precise reachability too (`memory.c:CheckFinalizers`),
so such an object is finalized by GNU R at the same collection". It is
honest about the limit ("does not prove resource safety for unprotected
C, which GNU R does not give either") and lists what remains to run
(package suites under gctorture with `ROBIGO_GC_PIN_REPORT=1`, a
premature-finalization probe, narrowing and zeroing as separate
switches). My `PIN_AUDIT` is now `ROBIGO_GC_PIN_REPORT=1` (191-194, §16).

##### B9. M1 not specified enough to start: **partly fixed**

Fixed:

- The M1 row (618) names the base subset: "the base R functions
  `calls.R` reaches, loaded from the 4.6.2 sources listed in
  `crates/robigo-rt/proto/base-subset.txt` (`conditions.R`,
  `stop`/`warning`, `structure`, `new.env`, `cat`, ...), with their
  internals in Rust; a minimal `robigo-rtapi` for the E6 cases".
- §15.2 has the "decided in" column (696-712).

Still open, two contradictions between M1's exit and the steps that own
the work:

1. **M1 needs appendix rows that M2 owns.** M2 (619) is "heap changes
   (appendix) in `robigo-heap`". But M1 already needs four kinds of
   appendix work:
   - safepoint collection, which is the appendix's `allocation` row;
   - the collection order, which is the `collect_needing` row;
   - value-stack and context roots, which is the `roots` row;
   - tracing of slot frames, overflow slots and `PROMISE_TAG` (M1 builds
     slot frames and promises), which is the `trace` row.

   E3 (723) also requires "`ROBIGO_GC_MOVE=1` clean" at M1, which is the
   `debug` row. Only `layout` (D8, D13, both "decided in M2") is M2 work.
2. **E6 needs `dyn.load`, `.Call` and printing, which M1 does not
   list.** The four E6 capi cases (`tests/corpora/capi/cases/{eval,
   pairlist,unwind,extptr}.R`) start with `dyn.load("x.so")`. They also
   `print` pairlists, calls (deparse), external pointers and weak
   references, and they use `tryCatch`, `paste`, `identical` and `gc()`.
   M5 owns `dyn.load`, and M1's base subset has `cat` but no `print`.

**Replacement text.** M2 row "what":

> the appendix's `layout` row (D8, D13) in `robigo-heap`, with tests and
> gctorture

Append to the M1 row "what":

> the appendix's `allocation`, `collect_needing`, `roots`, `trace`,
> `debug` and `removed` rows in `robigo-heap`; `dyn.load` of one shared
> object, `.Call` by symbol name, and `print`/deparse for the value types
> the four E6 cases print (pairlists, calls, external pointers, weak
> references, character and integer vectors)

#### 3. New blocking findings (introduced by v2)

##### N1. §2.4: "allocation-free" is false for deferred coercions (§2.4 97-99, §2.2 70-72, §8 392)

**Problem.** v2 reads: "Token-free reads are header and node fields of
non-ALTREP objects and of robigo's own ALTREP classes, whose methods are
Rust, allocation-free and non-signalling." Muse's round-3 text, which
this comes from (`muse-r3.md:402-403`), said "allocation-only". The
rewrite turned it into "allocation-free", and the result is false for one
of §8's own classes: "deferred coercions".

The deferred string class's element method does three things:

- it allocates the expanded STRSXP on first use;
- it allocates the element's CHARSXP;
- it writes both back into the ALTREP object
  (`R450/src/main/altclasses.c:685-698`, `ExpandDeferredStringElt`:
  `allocVector`, `SET_DEFERRED_STRING_EXPANDED(x, val)`, then
  `SET_STRING_ELT`).

It also reads isolate state (`R_print.digits`, `R_print.scipen`, `:708`).
Under §2.2 (70-72), "mutate an existing object takes `&mut Cx`". As
written, robigo has two choices, and both break the doc:

- `STRING_ELT` on a deferred coercion takes `&mut Cx` on every element,
  which contradicts §2.4's token-free claim and makes string loops slow;
- or the method writes under `&Cx`, which contradicts §2.2.

Allocation is not the problem. §3 makes allocation under `&Cx` sound,
because it never collects. The cache write is the problem.

**Replacement for §2.4's first sentence:**

> Token-free reads are header and node fields of non-ALTREP objects and
> of robigo's own ALTREP classes, whose methods are Rust, never collect,
> signal or run R code, may allocate (§3), and may write only their own
> private cache (the expanded data in `data2`) through the heap's
> barrier. A slice (`as_slice`) exists only for storage that no method
> will write again (standard storage, or an own class once fully
> materialized).

Append to the §2.2 list "Anything that ... mutate an existing object
takes `&mut Cx`":

> (except an own ALTREP class filling its private cache, above)

##### N2. The per-DLL strong-root policy contradicts §0 and §17 (§3 step 3 173-174, §17 item 2 764-767)

**Problem.** §3 step 3 says "A per-DLL policy can make one library's
segments strong roots (§17)". §17 item 2 says, in the same item: "No
cleverer mark: any rule that lets a stack word delay a finalizer brings
back the stale-word misses." A strong-root segment is exactly such a
rule. It delays finalizers that GNU R would run, and not only for the
buggy object: it delays them for every correctly protected object of that
DLL whose stale word stays in the scanned frames. That is the 30/0/0
pattern from §15.1. §0 (31-32) forbids an inexact variant as a
candidate. Fable proposed the switch (`fable-r3.md:159-160`). The merge
adopted it as a policy without reconciling it with §0.

**Replacement for the sentence in §3 step 3 and the clause in §17 item 2:**

> `ROBIGO_GC_STRONG_DLL=<name>`, a diagnostic only, makes one library's
> segments strong roots to confirm a `ROBIGO_GC_PIN_REPORT=1` finding (if
> the failure disappears, the package lacks a `PROTECT`). It is never a
> default, never set by robigo for a package, and no ratchet case may
> depend on it.

##### N3. Base-package natives vs L4 (§10 504-505, M8 625)

**Problem.** L4 (41) says "the end state contains no GNU R C". §10 (504-505)
says base-package natives are "built from GNU R's sources now, replaced by
Rust where measured faster and exact". That allows stats, utils and
graphics C to remain wherever a port is not yet faster. M8's exit (625)
is "zero C builtins", and base-package natives are not builtins. So no
milestone enforces L4 for them.

**Replacement for §10's bullet:**

> **Base-package natives** (stats, utils, graphics, ...): built from GNU
> R's sources now; each is replaced by a Rust port that is exact and
> faster, in order of measured share of package-suite time, all of them
> by M8.

M8 "what":

> zero GNU R C in the build (no C builtins, no base-package natives)

#### 4. Non-blocking (at most five)

1. **cmp.R's decisions must be the `Bc` profile's decisions, made at the
   same moment** (§9.1 424-427). cmp.R's inlining depends on the
   environment at compile time. It checks whether a base function is
   shadowed, and it reads the `optimize` level and the compiler options.
   §9.1 runs robigo's own compiler when GNU R's JIT would compile, then
   runs cmp.R later "on observation". If a binding changed in between, the
   printed or serialized BCODESXP can differ from the code that ran. The
   two compilers can also disagree outright. Text:

   > the `Bc` profile's inlining and frozen-binding decisions are
   > cmp.R's: at compile time robigo records the bindings and options
   > cmp.R would consult, and the BCODESXP made on observation is
   > compiled against that record

   `ROBIGO_MATERIALIZE`-style checking can then compare the two.
2. **D0's bar is not written down** (§10 484-486: "Adopt `robigo-cbase`
   only if the boot set is large and the adapter cost is under E1's
   bars"). §15.2 (692) requires bars "written before the runs". Name both
   numbers, for example "boot set > N builtins *and* adapter overhead
   < X instructions per call over the Rust path on the three files".
3. **How builtins below `rt` call the evaluator** (§2.4 115-117: "heap →
   rt-core facade → builtins → rt"). `lapply`, `do.call`, `sort` with a
   comparator, and every internal generic's `try_dispatch` need `eval`,
   `apply_closure` and dispatch, which live in `rt`. Say that the facade
   holds a hook table that `rt` installs in `Cx` at startup (eval, apply,
   dispatch, force). Then measure the indirect-call cost on `lapply` and
   `[` dispatch in E6.
4. **§13's tie rule does not say whether a tie passes** (566: "two
   runtimes within 2% tie"). The bars say "robigo fastest of all
   runtimes". State that a tie does not count as fastest, or that it
   does, so that "fastest on every benchmark" can be decided.
5. **L6's wording vs §5.3.** L6 (43) says "no process-global mutable
   state". §5.3 (269) has a process-shared symbol arena behind one lock.
   `docs/core.md:994-997` (Decision A.2) explicitly allows this as "the
   one shared mutable structure". Quote that exception in L6's row, so
   that it does not read as a violation.


### Muse

### Council round 4 review of `docs/g3.md` v2 (Muse Wonder Full)

Reviewer: Muse Wonder Full, round 4. Scope per `brief-r4.md`: re-review
the revised G3 design against my round-3 blocking findings (B1–B14 in
`muse-r3.md`); new blocking findings only where v2 introduced them;
read-only. `expA-results.md` was present this round and is used for B10.
Citations to the design are `docs/g3.md:<line>` in the worktree;
`R462/` is the GNU R 4.6.2 tree in the scratchpad. One verification I
ran: `docs/g3-council.md` does not exist in the worktree yet (relevant
to B4).

#### 1. Score: 7.5/10

| aspect | score |
|---|---|
| Direction (one heap, plan B, safepoint collection, GNU REFCNT, 4.6.2, measurement gates) | 8/10 |
| Correctness of the doc as a merged, self-consistent spec | 7/10 |
| Specific enough to start M1 without inventing mechanisms | 8/10 |

Round 3 was 5/10, with "fix B1–B9 and this is an 8". V2 fixes eleven
of my fourteen blocking findings outright, most of them in the words
proposed (which is fine — they were written to be liftable). The three
partly-fixed items (B4, B5, B10) and the one new unsound sentence the
rewrite introduced (NB1) are all one-paragraph repairs, and none of
them is a reason to delay M1 itself: B4's deviation list and NB1's
sentence belong in the same pull request that lands this doc, B5 gates
M3, B10 gates the heap's adoption of pins in M2. I hold the score at
7.5 rather than the promised 8 only because B4/B5 are inside the
B1–B9 set and are not quite closed; with the four replacement texts
below applied verbatim this is an 8, and M1 can start now in parallel
with them.

#### 2. Round-3 blocking findings, one by one

##### B1. `Cx` branding under re-entrancy — **Fixed**

> "`Cx` is a `!Send + !Sync` capability over `*mut Isolate`; the
> isolate's fields live in `UnsafeCell`s … every entry into R or C code
> consumes it, ends all field borrows, and re-creates it from the raw
> pointer on return" (`docs/g3.md:57-61`); "What the borrow checker
> does *not* prove: that allocation never collects (an audited heap
> invariant, checked by `ROBIGO_GC_MOVE=1`)" (`docs/g3.md:73-75`); "A
> `Ref` may be minted only from a `Root`, `Args`, a `Global`, a field
> of an object so reached within the same interval, or a fresh
> allocation within the same interval" (`docs/g3.md:66-69`).

All three gaps are closed, including the appendix row splitting today's
collecting allocator into a poll-only path (`docs/g3.md:124-129,777`)
and the compile-fail case for "a slice across a callback"
(`docs/g3.md:76-77`).

##### B2. What a `Root` is — **Fixed**

> "Roots live in the isolate's **value stack** (§9), never on
> `R_PPStack`: C may `UNPROTECT` across a callback, and context
> unwinding restores `R_PPStackTop`" (`docs/g3.md:81-83`); "`Root<'s>`
> is an index into that stack branded by a generative scope; slot
> addresses are stable because the range is reserved, not grown. Every
> builtin and native dispatch and every unwind saves and restores the
> stack top … on every path including `Err`" (`docs/g3.md:84-88`).

The value stack also appears as item 1 of the mark list
(`docs/g3.md:164-165`), and the dead-slot hazard is D9
(`docs/g3.md:91-93,708`).

##### B3. Deferred garbage inside one builtin or loop — **Fixed**

> "Any Rust loop whose allocation is not bounded by a constant calls
> `cx.poll_alloc(bytes)` per chunk, with its inputs in roots and slices
> reacquired after it. The chunk bound is part of E4; a builtin that
> cannot name its chunk does not merge. Interrupt latency in long
> kernels uses the same poll" (`docs/g3.md:140-144`); "C API exports
> that may collect do so on entry when due, before allocating, and
> **root their `SEXP` arguments precisely for their duration**"
> (`docs/g3.md:152-155`).

The `LARGE`/`alloc_rooted` split (`docs/g3.md:134-139`) covers the
single-large-allocation case, and E4 is redefined into something
measurable with a plain allocation counter: "bytes allocated between
`GC_DUE` being set and the collection that serves it never exceed
max(32 MiB, 10% of the live heap)" (`docs/g3.md:724-726`). The chunk
bound's numeric value is correctly left to E4, as proposed.

##### B4. Slot frames: growth, encoding, order, varloc — **Partly fixed**

What is fixed: the normative reference and every mechanism are now
named.

> "The normative detail (word map, key bits, overflow, shape
> transitions, rank, expand on read) is `fable-r2.md` §3.1-3.2 in
> `docs/g3-council.md`, amended by this section" (`docs/g3.md:223-226`);
> "One ENVSXP with inline typed slots plus an **overflow area**"
> (`docs/g3.md:230`); "**cell** (the slot points to a GNU R binding
> cell … the binding stays cell-backed for its lifetime, because
> 4.6.2's `R_varloc_t` is `{ SEXP cell }`)" (`docs/g3.md:241-245`);
> "Each binding carries a definition stamp … enumeration lists by
> stamp descending" (`docs/g3.md:249-252`); "**Expand on read** …
> boxed once and stored back" (`docs/g3.md:253-255`).

Residual: two parts of the proposed fix were not taken. (a)
`docs/g3-council.md` is not in the worktree (verified); the doc's own
status line promises it "with this doc's pull request"
(`docs/g3.md:3-6`), but M0's exit (`docs/g3.md:617`) does not name it,
so as reviewed the normative reference dangles. (b) The deviations
from Fable's text are not listed as deviations, and they are real:
Fable's growth mechanism is shape-transition-first with overflow as
the spill ("Defining a symbol the shape lacks moves the frame to the
child shape and uses the next slot (the allocation's slack, then
`overflow`)", `fable-r2.md`, §3.1), where v2 makes overflow the
default and transitions the measured variant D2s; Fable's varloc is
"`{env, slot address}` in robigo's `Defn.h`" (same section), where v2
keeps GNU's `{SEXP cell}`. "Amended by this section" establishes
precedence only where §5 happens to speak; an implementer still has
to diff two long texts to find every place they conflict — a smaller
version of the round-3 fault.

Smallest replacement for `docs/g3.md:223-226`:

> The normative detail (word map, key bits, overflow, shape
> transitions, rank, expand on read) is `fable-r2.md` §3.1-3.2 in
> `docs/g3-council.md`, which lands verbatim in the same pull request
> as this doc (M0 does not close without it). Deviations from it,
> exhaustive: the overflow area is the default for names outside the
> shape and shape transitions are the measured variant (D2s); listing
> order is per-binding definition stamps, not shape history;
> `R_varloc_t` stays GNU R's `{ SEXP cell }` and a varloc binding
> becomes cell-backed permanently (not `{env, slot}`); slot kinds add
> unbound and cell to Fable's four. Where this section and §3.1-3.2
> conflict outside that list, §3.1-3.2 wins and the conflict is a bug
> in this doc.

##### B5. GNU bytecode (BCODESXP) — **Partly fixed**

What is fixed: BCODESXP exists, is kept and translated; the `Bc`
profile is GNU's compile *decisions*, tracked per closure; writes
invalidate.

> "**BODY may be a BCODESXP** (from `cmpfun`, lazy-load databases,
> saved closures); its first constant is the source; robigo keeps the
> BCODESXP object for `identical`, `disassemble` and serialization,
> and translates it to the IR on first call" (`docs/g3.md:286-289`);
> "**Which profile runs is GNU R's decision**, tracked per closure in
> the code object: a port of `eval.c:R_CheckJIT`, `JIT_score`,
> `MAYBEJIT`/ `NOJIT`, `R_JIT_STRATEGY` and the JIT cache"
> (`docs/g3.md:417-420`); "any write to BODY, FORMALS or CLOENV,
> R-level or C, drops the code object and the frame shape; `duplicate`
> of a closure drops the code object" (`docs/g3.md:292-294`).

Residual, in the one new mechanism v2 adds:

> "A GNU R-shaped BCODESXP is produced on observation (`print`'s
> `<bytecode>` line, `inspect`, `serialize`, `bodyCode`,
> `disassemble`) by running GNU R's compiler package on the closure's
> source" (`docs/g3.md:424-427`).

Two faults. (a) *Timing*: `cmp.R`'s decisions depend on the compile
environment in force when it runs. GNU R crystallizes them when its
JIT replaces BODY; running the compiler later, at observation, after
a rebinding of (say) `+`, can produce a different BCODESXP from the
one the code object's frozen `Bc` decisions (`docs/g3.md:415-416` —
"base bindings frozen at compile time") say is running — so
`disassemble` and `serialize` bytes can disagree with GNU R on
exactly the call-count-dependent cases §9.1 exists to reproduce. (b)
*Re-entrancy*: observation happens in places GNU R merely reads a
field — inside `serialize`, `inspect`, error reporting — where
running the compiler package (R code that allocates, can error, and
can in principle be redefined) is a new failure mode GNU R does not
have. Neither matters for M1; both matter before M3's evaluator
is "complete".

Smallest replacement for `docs/g3.md:424-427`:

> A GNU R-shaped BCODESXP is crystallized at the moment GNU R's JIT
> would replace BODY, at a call boundary, by running GNU R's compiler
> package on the closure's source in the compile environment and base
> bindings in force at that moment, and is stored in the code object.
> Observation (`print`'s `<bytecode>` line, `inspect`, `serialize`,
> `bodyCode`, `disassemble`) only exposes the stored object and never
> runs the compiler; a closure GNU R has not yet compiled has no
> BCODESXP to expose. The `bytecode-fidelity` fixtures carry over.

##### B6. The GNU C crutch — **Fixed**

> "**The C crutch is decided by D0**, before M3: count the distinct
> builtins a base boot needs (on the old runtime); prototype the
> adapter (VM `Args` → GNU R's `(call, op, args pairlist, env)`
> signature, promise-shaped cells for evaluated builtins, visibility,
> a per-call trampoline) for `arithmetic.c`, `subset.c`, `coerce.c`;
> measure instructions per call against Rust. Adopt `robigo-cbase`
> only if the boot set is large and the adapter cost is under E1's
> bars" (`docs/g3.md:479-486`); "(The stats precedent, fastr 30,576 vs
> 30,311 cases, is package natives behind `.Call`, not `src/main`
> builtins: it shows the build technique, not this adapter.)"
> (`docs/g3.md:492-494`).

The ABI manifest, the `Defn.h` definitions and the allowlist check
(`docs/g3.md:487-491`), plus L4's "adopted only if D0 shows it pays"
(`docs/g3.md:41`), close all four sub-problems. It is correctly
presented as undecided rather than as consensus.

##### B7. Differential mode soundness — **Fixed**

> "**Differential mode**: the registry gives each builtin an effect
> class. `pure`: `ROBIGO_BUILTIN_DIFF=1` runs both implementations
> in-process on duplicated arguments, with warnings and errors
> captured as records, and fails on any difference. `rng`: the same,
> with generator state saved and restored. Everything else, and
> anything unclassified, is compared out of process"
> (`docs/g3.md:496-501`), including "results, post-state, output,
> warnings, callback order and external effects" (`docs/g3.md:501-503`).

The unclassified-defaults-to-effectful rule is there. Good.

##### B8. Heap-crate coexistence with the frozen runtime — **Fixed**

> "`robigo-rt` has its own heap and C API crates from M1.
> `crates/robigo-heap` is a copy of `robigo-core` taken when M1
> starts; every heap change lands there … `robigo-core`,
> `robigo-capi` and `robigo-runtime` are frozen (L2) and deleted at
> the flip … No cfg-gated second layout exists in any crate"
> (`docs/g3.md:605-611`), with L2 amended to freeze the three crates
> together (`docs/g3.md:39`).

Exactly the proposed fork, and the alternative (feature-gated v2) is
explicitly rejected rather than left open.

##### B9. M1 scope vs exit criteria — **Fixed**

M1 now owns what its exit measures:

> "S3 dispatch, the condition system's internals; the base R functions
> `calls.R` reaches, loaded from the 4.6.2 sources listed in
> `crates/robigo-rt/proto/base-subset.txt` … a minimal `robigo-rtapi`
> for the E6 cases" (`docs/g3.md:618`); "E1 `calls.R` (one process, JIT
> off), M1 rows: plain, defaults, named, `e$f`, `<<-`, S3 ≤ 1.0x GNU R's
> instructions, the plain-call idioms ≤ 0.6x … M3 adds `tryCatch`"
> (`docs/g3.md:716-718`); E6 runs "the capi cases `eval`, `pairlist`,
> `unwind`, `extptr` … over the minimal `robigo-rtapi`"
> (`docs/g3.md:732-735`).

That resolves the contradiction both ways (scope widened *and* rows
split), and naming a checked-in `base-subset.txt` makes the base subset
an artifact instead of a hand-wave.

##### B10. Experiment A deciding pins vs strong roots — **Partly fixed**

What is fixed is substantial. The results now include a discriminating
planted-word test and the decision is argued from GNU-equivalence, not
from a delay count pins would win by construction:

> "capi `native_call_frames_keep_nothing` (a dead pointer planted in
> every word of a native call's frames): fails with the stub off and
> the old order, passes with the stub off and pin-only" and stress
> runs "baseline 0/0/0; pin-only 0/0/0; stub off, old order 30/0/0;
> pin-only and stub off 0/0/0" (`docs/g3.md:657-663`); "GNU R decides
> finalizers on precise reachability too (`memory.c:CheckFinalizers`),
> so such an object is finalized by GNU R at the same collection and
> its memory freed; pins give GNU R's finalizer decision in every case
> with memory that lives longer … The experiment shows compatibility
> on the tested cases; it does not prove resource safety for
> unprotected C, which GNU R does not give either"
> (`docs/g3.md:675-682`). I accept that argument: for the finalizer
> *decision*, pins reproduce GNU R exactly and strong roots are the
> deviation. My round-3 safety objection is answered in the text
> (`docs/g3.md:673-675` records the 2–2 split honestly).

Residual: the decision is recorded as taken — "**Decision: pins** (§3
step 3)" (`docs/g3.md:672`), §3 step 3 and the appendix are normative
(`docs/g3.md:170-174,778-779`) — while the three validations that were
the missing half of the round-3 design are still only listed as to-do,
with no pre-registered rule saying what result blocks adoption, in
contrast to §15.2's own "Bars are written before the runs"
(`docs/g3.md:692`):

> "Still to run before the heap adopts it: the capi corpus and the
> package suites with natives built, under gctorture, with
> `ROBIGO_GC_PIN_REPORT=1`; a premature-finalization probe (an
> unprotected external pointer passed to an allocating export is not
> finalized during the call, as in GNU R); narrowing and zeroing
> measured as separate switches" (`docs/g3.md:682-687`).

"Before the heap adopts it" gives the sequence but not the bar: if a
package suite that passes today fails under pins, is that package
moved to per-DLL strong roots (`docs/g3.md:173-174`), or is the global
decision reopened? §17 item 2 hints at the former but does not bind
it. This gates M2, not M1.

Smallest replacement, appended after `docs/g3.md:687` (before "The stub
and the zeroing go…"):

> Adoption rule for those runs, written before them: `robigo-heap`
> adopts pins only if no capi-corpus or package-suite case that passes
> today fails under pins, and the premature-finalization probe
> passes. A package that fails only under pins gets per-DLL strong
> roots (§3 step 3), is named in `docs/ledger/g3.md`, and its cases
> must pass under that policy; any other failure reopens this
> decision. Narrowing and zeroing are removed only on their separate
> measurements, each of which must leave both probes unchanged.

##### B11. Single pending-exit slot — **Fixed**

> "`RResult` carries a zero-sized error token; the exit (kind, target
> context, mask, value, condition, call) is the top record of a
> **pending-exit stack** in the isolate, a precise root. Code that runs
> during an exit (`R_run_onexits`, a `cend`, an `R_UnwindProtect`
> cleanup, a calling handler, a finalizer) parks the record in the
> context being left (GNU R's `jumptarget`, `jumpmask`,
> `R_ReturnedValue` …) and restores it afterwards, unless that code
> leaves by its own exit beyond it, which supersedes it as in GNU R"
> (`docs/g3.md:359-366`), with E8 testing all three nestings "in every
> tier and across a C frame" (`docs/g3.md:739-742`).

Parking in the GNU context record is better than the stack alone that
I proposed, and the size overclaim is withdrawn ("Sizes are asserted
for the hot-path result types, not promised for every `T`",
`docs/g3.md:368-369`).

##### B12. Token-free reads vs package ALTREP; `regions()` — **Fixed**

> "Token-free reads are header and node fields of non-ALTREP objects
> and of robigo's own ALTREP classes, whose methods are Rust,
> allocation-free and non-signalling. **Length, data pointer and
> element access on any other ALTREP take `&mut Cx`** on a root"
> (`docs/g3.md:97-101`); "`regions_into(&mut [T; N])` fills a caller
> buffer for generated classes (GNU R's `ITERATE_BY_REGION` shape)"
> (`docs/g3.md:105-107`), plus "Anything that can … call a package
> ALTREP method … takes `&mut Cx` and ends every `'g`"
> (`docs/g3.md:69-72`).

The `Ref::len() -> Option` / `cx.xlength(root)` split
(`docs/g3.md:101-103`) gives the safe API an honest shape.

##### B13. Argument/binding state machine — **Fixed**

> "**A slot's kind is the kind GNU R's binding would have at the same
> moment, never the cheapest one**" (`docs/g3.md:298-299`); "An argument
> may itself be a PROMSXP (forwarded `...`, made by C,
> `delayedAssign`); binding-kind queries unwrap promise chains as
> `envir.c:840-856` does" (`docs/g3.md:311-313`); the §6.3 table
> (`docs/g3.md:320-328`) gives force / assign / `rm` / C-query /
> materialize per kind, with "Each cell is ported from
> `envir.c`/`eval.c`, and its REFCNT effect is GNU R's
> (`unpromiseArgs`'s decrements become the forced-slot release on call
> exit)" (`docs/g3.md:330-332`); "A memoized plan records which partial
> matches it used and signals the same warnings `matchArgs` would under
> the options in force at that call" (`docs/g3.md:338-340`).

A value slot holding a PROMSXP falls out of the governing rule: its
kind *is* delayed or forced by the `BINDING_TYPE` derivation the doc
cites (`R462/src/main/envir.c:807-829`, `promiseUnwrap` at
`:786-805`), so the table's delayed/forced rows govern it after
unwrapping. The precomputed-delayed row (caller evaluated early,
reported delayed until first read) is the correct GNU-relative state,
and its materialization cell ("PROMSXP unforced, value attached on
force") is coherent precisely because the precomputed value stays
private to the slot until force.

##### B14. Sole-isolate global and symbol arena vs Decision A.2 — **Fixed**

> "L6 | `docs/core.md` §10 Decision A.2 stands: several isolates per
> process, all mutable state behind one thread-local, no
> process-global mutable state (a sole-isolate global is not a
> candidate)" (`docs/g3.md:43`); "The symbol arena is
> **process-shared** (L6): one lock, append-only segments never
> reallocated, 64-byte-aligned symbols, ids by arena arithmetic and
> global. Per isolate: the symbol's value cell … in tables indexed by
> the global id" (`docs/g3.md:269-273`).

D7 is gone from the decision table; §11 spends one TLS read
(initial-exec in C) as A.2 requires (`docs/g3.md:527-529`).

#### 3. New blocking findings introduced by v2

##### NB1. §3 item 5: the restartable-builtin criterion is unsound as written

The (new, welcome) allocation-failure discipline states its invariant
and then a test for it that does not imply it:

> "The caller releases views, keeps its state rooted and retries only
> the failed allocation through `&mut Cx` …; no committed effect is
> replayed. Builtins the registry marks restartable (no effect before
> their first allocation) may simply be re-run" (`docs/g3.md:147-151`).

"No effect before their *first* allocation" permits effects after the
first allocation and before the one that fails: a builtin that
allocates an intermediate, then touches a package ALTREP element
(a C callback), signals a warning, or draws RNG, and only then hits
`Err(NeedGc)` on a later allocation, would be re-run and would repeat
that effect — violating the sentence immediately before it and E9's
own test ("an injected allocation failure after a visible effect
neither loses nor repeats the effect", `docs/g3.md:743-745`). The
invariant and the test point in the right direction; only the
parenthetical is wrong, but a registry of ~760 builtins will be
classified against the parenthetical, so it is load-bearing.

Smallest replacement for the parenthetical at `docs/g3.md:150-151`:

> Builtins the registry marks restartable (no observable effect —
> store, warning, callback, RNG draw or native call — at any point
> before their commit point, hence before any allocation that can
> fail and be retried) may simply be re-run.

#### 4. Non-blocking items (five)

1. **§5.2 vs §8 wording.** "keep GNU R's representation bit for bit"
   (`docs/g3.md:261-262`) cannot mean physical bits once the header is
   24 bytes with per-type nodes (`docs/g3.md:399-400`); say "logical
   structure (FRAME/HASHTAB shape and traversal order)" to keep the
   census argument honest. Carried over from round 3 (X4), still only
   a wording risk.
2. **§3 item 7's "eval-count poll" is still undefined in a VM-only
   engine** (`docs/g3.md:158-160`). GNU R counts evaluations
   (`eval.c:1093-1099`) and also polls inside `bcEval`
   (`eval.c:6247`); finalizer timing relative to printed output is
   observable, so name the unit (VM calls? ops?) in §3 or §12 before
   M3 (round 3, X6).
3. **§17 item 4 overstates §12's extension.** Three round-3 rows did
   not land: interrupts as an observable (non-catchable jump, timing
   against output; mechanism is covered at `docs/g3.md:143-144` but
   there is no §12 row), `fork`/`mclapply` constraints on the new
   layout (the value-stack reservation and the shared symbol arena
   are new surface for `parallel::mclapply`; `grep fork docs/g3.md`
   finds only the heap fork), and S4 dispatch/`methods` invalidation
   (§13 measures S4 speed at `docs/g3.md:587` but §12 has no row for
   its semantics). Extend §12 or trim §17's claim.
4. **`ROBIGO_REF_EVAL=1` has no CI job.** §3's GC checks end with "All
   are CI jobs on the `rt` slice" (`docs/g3.md:189-194`); §16's table
   lists the reference interpreter as a verification mode
   (`docs/g3.md:758`) with no such sentence. During M3 it is the
   one-engine design's only independent semantic oracle
   (`docs/g3.md:428-429`); an un-CI'd oracle bit-rots into agreement
   with the engine. One sentence in §16 naming its CI slice (and the
   same for the quarantine/poison modes) closes it.
5. **M7 still flips and deletes in one PR** (`docs/g3.md:624`), as in
   v1. Flipping the default in one PR and deleting
   `robigo-runtime`/`robigo-core`/`robigo-capi`/`rlib/` in a follow-up
   keeps L1's intent and makes the highest-risk step bisectable.

#### Closing

Start M1. Everything M1 builds first — `Cx` and branding, roots in the
value stack, safepoint collection with the poll rule, slot frames with
overflow/stamps/cell, argument-slot kinds with the §6.3 table,
contexts with the pending-exit stack, the `robigo-heap` fork, and exit
criteria that match the scope — is now specified at the level an
implementer needs, which was not true at round 3. The four
one-paragraph repairs above (B4's deviation list, B5's crystallization
point, B10's adoption rule, NB1's parenthetical) should ride in the
pull request that lands this doc and `docs/g3-council.md`; none of them
waits on new evidence, and with them my score is 8/10.


## Round 5

### Brief

### Council round 5: confirm the round-4 fixes

Same rules as before (read-only; no subagents; no builds into the repo).
`docs/g3.md` (worktree, uncommitted) is now v3: the round-4 replacement
texts from all four reviews (`*-r4.md` in this directory) were applied,
mostly verbatim. Two were adapted: Muse's pin adoption rule no longer uses
per-DLL strong roots as an escape (Opus N2 made that a diagnostic only, so
any failure reopens the decision), and D0's bar now has numbers (boot set
≥ 150 builtins not already Rust, adapter ≤ 1.10x a Rust port's instructions
per call; pilot files `coerce.c`, `bind.c`, `paste.c`).

Deliverable, in the file named in your task message, short:

1. **Score out of 10.**
2. **Each of your round-4 open items: fixed or not**, one line each, with
   the v3 text for anything not fixed and the smallest replacement.
3. **Anything v3's edits broke** (a contradiction they introduced). No new
   scope.

Final chat reply: the path, the score, and open items (or "none").


### Astra

### Astra: council round 5

**Score: 9/10.** All three round-4 blocking items are fixed. One v3 edit introduces a compatibility-contract contradiction. Two earlier non-blocking follow-ups remain. This was a read-only document review; no builds, experiments or repository edits.

#### Round-4 open items

- **B2, root ownership — fixed.** §2.3 enforces ownership in every build, requires the owning `&mut Cx` for root replacement, preserves scope storage independently, and names the release-mode and lifetime tests.
- **B5, differential execution and forced-copy identity — fixed.** §10 excludes RNG and callback-capable paths from same-process replay and compares final RNG state; E5 gives deliberate native alias mutation separate production-mode oracle expectations. “Actual inputs and execution path” must include argument duplication, as requested in round 4.
- **B6, allocation retry — fixed.** §3 item 5 conditions restartability on the particular failed allocation, after checking for committed effects; allocator failure also has its own result type.
- **Follow-up 1, named B1/B4 regressions — not fixed, non-blocking.** §2.4 still gives the ALTREP rule and §9.3 the mutation rule, but neither names the requested cases. Smallest addition to E6: “Test a package ALTREP Length callback that allocates and re-enters R, in-place mutation of BODY/FORMALS children, and mutation during a callback while compiled code is suspended.”
- **Follow-up 2, pin diagnostics and resource probe — partly fixed, non-blocking.** §15.1 correctly calls reports candidates, adds the external-buffer probe, and makes strong roots diagnostic only. §3:221–222 still says “(a missing `PROTECT` in that package)”; replace with “(a candidate missing `PROTECT`, not proof; see §15.1)”. Likewise change §16’s pin-report description from “packages that rely on an unprotected C local” to “candidate unprotected C locals; see §15.1”.
- **Follow-up 3, torture modes — fixed.** §3 expressly distinguishes debug safepoint torture from evidence of R-level `gctorture(TRUE)` equivalence.

B1, B3, B4 and B7 remain fixed; their accepted contracts survive v3.

#### Contradiction introduced by v3

**§3’s new GC exclusions contradict §0’s compatibility contract.** §0:16–22 requires “every behaviour GNU R 4.6.2 makes observable, as bytes” and explicitly includes `gc()`. §3:210–214 now says:

> *Not compared*, like addresses […] cell counts in the `gc()` table, trigger sizes, the number and level of collections, `gcinfo(TRUE)` report contents.

These are observable numerical values and output, not inherently variable addresses or times. Different internal representations do not by themselves authorize different exposed values. The earlier contract required revising the policy when exact observations could not be kept; this edit silently narrows that requirement.

Smallest replacement for those two sentences, preserving the lead’s stated goal:

> Cell counts in the `gc()` table, trigger sizes, collection counts and levels, and `gcinfo(TRUE)` report contents remain oracle-tested observables. The new representation must reproduce their GNU R-visible values; if it cannot, the design must be revised rather than exempting them from comparison.

If the intended resolution is instead to relax compatibility, that needs an explicit lead decision and a matching change to §0. I do not infer that authorization from another reviewer’s suggested exemption.

No other new contradiction found within this round’s scope. M1 work can proceed, but the document should resolve this compatibility claim before approval.


### Fable

### G3 round 5: confirmation of `docs/g3.md` v3 (Claude Fable 5.2)

Base: main `4cff0ceb` plus the uncommitted `docs/g3.md` (v3, 869 lines).
Read-only; nothing was built or run. `g3.md:N` is a line of v3. `R462/` is
`.../scratchpad/r462/R-4.6.2/src`. Source lines cited below were read in
this round.

#### 1. Score

**9.5 / 10** (round 4: 9).

All four of my open blocking items are fixed. Of the five non-blocking
ones, three are fixed, one is partly fixed (D0's pilot files) and one
sub-item is unchanged (D1's third variant). The edits introduced seven
small inconsistencies (section 3). One of them (S1) changes what M1
builds and needs one sentence before M1 starts. The rest are one-line
corrections that block nothing.

#### 2. Round-4 open items

##### Blocking

- **B3 (experiment A's runs had no milestone): fixed.** M0 owns the runs
  and its exit names the adoption rule (`g3.md:684`); §15.1 says "These
  runs belong to M0" (`g3.md:756`). One consequence: S3.
- **N1 (`duplicate` and invalidation dropped GNU R's JIT state): fixed.**
  `g3.md:328-336` is my text; §9.1 says "per closure and per body"
  (`g3.md:466-467`). The citations hold: `R462/main/duplicate.c:309-310`
  copies `NOJIT`/`MAYBEJIT`; `builtin.c:322-325` restores the source body
  and leaves the bits (the doc says `323-325`; the test is on `322`).
- **N2 (missing-default row): fixed.** Two rows at `g3.md:368-369`; the
  oracle check is in E5 (`g3.md:812-816`).
- **N3 (`TRUELENGTH` scratch on shared CHARSXPs): fixed.**
  `g3.md:447-452`, with the branch in D8's bar; §7 covers the cached
  symbol word (`g3.md:426-428`).

##### Non-blocking

1. **Collection observables: fixed** (`g3.md:205-215`). One consequence:
   S5.
2. **Word map indicative, three constraints, `g3-council.md` gated on M0:
   fixed** (`g3.md:252-256`). One stale phrase: S6.
3. **M1/M2 overlap: fixed.** M1 takes the appendix's allocation,
   `collect_needing`, roots, trace, debug and removed rows; M2 takes the
   layout row (`g3.md:685-686`).
4. **Three round-3 items: two fixed, one not.**
   - Finalizer call sites: fixed (`g3.md:174-178`). `eval.c:1096`, `:6247`
     and `memory.c:3093, 3101` are the four calls.
   - Root list (`R_alloc` stack, objects awaiting finalization, derived
     trace): fixed (`g3.md:182-187`).
   - D1's third variant: **not fixed.** v3: "| D1 | collection |
     safepoint-only; Muse's auto-rooting allocator (fallback) | M1 |"
     (`g3.md:778`). Smallest replacement: "safepoint-only; Muse's
     auto-rooting allocator (fallback); Astra's `&mut Cx` allocation into
     an `Out` slot (control)". Or add one clause saying why it is out.
5. **Bars with no number or type: partly fixed.**
   - `Result<_, NeedGc>`: fixed (`g3.md:156-163`).
   - D0's numbers: fixed (`g3.md:539-543`).
   - D0's pilot files: **`coerce.c` has the defect I reported for
     `arithmetic.c` and `subset.c`.** v3: "for `coerce.c`, `bind.c` and
     `paste.c` (files the crutch would serve)" (`g3.md:538-539`). Most of
     `coerce.c` is in §10's designed-away list (`g3.md:520-524`): `as.*`
     (`do_asatomic`, `do_asvector`), `is.*` (`do_is`, `do_isna`, ...),
     `quote`, `substitute`. The crutch would serve only `typeof`,
     `as.call`, `as.function.default`, `str2lang` and `storage.mode<-`
     from it. `bind.c` also holds `c` (`names.c:209`), which is designed
     away. Smallest replacement: "for `attrib.c`, `bind.c` (`unlist`,
     `cbind`, `rbind`) and `paste.c`". `attrib.c` has evaluated builtins,
     internal dispatch and replacement functions, so it also tests the
     adapter on the link counts.

#### 3. What v3's edits broke

- **S1. §9.1 now reads as if the `Bc` profile's decisions come from
  running the compiler package (`g3.md:473-478`). Fix before M1.**
  - v3: "crystallized ... by running GNU R's compiler package ... The
    `Bc` profile's inlining and frozen-binding decisions are cmp.R's,
    recorded at that same moment."
  - The bullet above it says the decision is "a port of ...
    (today's `crates/robigo-runtime/src/vm/rjit.rs`)" (`g3.md:467-469`),
    and §9.2 carries that port over (`g3.md:497-498`).
  - M1 runs both profiles and E5 tests "after GNU R's JIT threshold", but
    M1 loads only `base-subset.txt` (`g3.md:685`). The compiler package
    cannot run there.
  - Smallest replacement for the second sentence: "The `Bc` profile's
    inlining and frozen-binding decisions come from the port
    (`vm/rjit.rs`), recorded at that same moment. From M3, CI checks them
    against what the stored BCODESXP inlined. M1 runs the port and makes
    no BCODESXP."
  - Not a contradiction, but a new cost: today's default build never runs
    the compiler ("it writes plain bodies unless built with
    `bytecode-fidelity`", `rjit.rs:24-26`). v3 runs it for every closure
    GNU R compiles. GNU R pays this too. §13's cold-code row and D11
    should count it.
- **S2. §12 still says "a BCODESXP on observation"** (`g3.md:607`), while
  §9.1 says observation "never runs the compiler" (`g3.md:478-480`).
  Replace with "the stored BCODESXP (§9.1)".
- **S3. M0 says "the two switches"; §15.1 now asks for more**
  (`g3.md:684, 749-756`).
  - The remaining runs need `ROBIGO_GC_PIN_REPORT=1`, narrowing and
    zeroing as separate switches, and `ROBIGO_GC_STRONG_DLL` to confirm a
    hit.
  - The tree has two switches. `ROBIGO_CALL65_PLAIN` turns narrowing and
    zeroing off together (`crates/robigo-core/src/shim.c`, uncommitted).
    The pin report and the per-DLL switch do not exist.
  - All of this is in `robigo-core`, which L2 freezes.
  - Smallest replacement in M0: "experiment A's remaining runs (§15.1) on
    main, with the experiment switches (pin-only, narrowing, zeroing, the
    pin report) on an unmerged branch of `robigo-core`".
  - The adoption rule compares "a case that passes today" with a run
    under gctorture (`g3.md:758-760`). Add "under the same mode", or a
    missing root that gctorture finds today will count against pins.
- **S4. The appendix removes narrowing and zeroing without condition;
  §15.1 makes it conditional.** Appendix: "the `.Call` stub, narrowing
  and zeroing removed" (`g3.md:865`). §15.1: "removed only on their
  separate measurements" (`g3.md:765-766`). M1 builds that appendix row.
  Replace with "removed where §15.1's measurements allow".
- **S5. §0 lists `gc()` among observables compared "as bytes"
  (`g3.md:19`); §3 now says its cell counts are not compared**
  (`g3.md:210-214`). In §0, extend "(addresses, elapsed times)" to
  "(addresses, elapsed times, the collection counts §3 lists)".
- **S6. Two stale phrases.**
  - "encoding per the normative layout" (`g3.md:276`), though the word
    map is now indicative (`g3.md:253-254`). Replace with "encoding per
    the word map M1 fixes (§5)".
  - §3's checks call a pin-report hit "a missing `PROTECT` in that
    package" (`g3.md:221-222`); §15.1 calls it "a candidate missing
    `PROTECT` to investigate" (`g3.md:763-764`). Use §15.1's wording in
    §3. The pin-report CI job then reports and does not gate.
- **S7. §16's table is cut in two.** The sentence "Every mode in this
  table has a CI job on the `rt` slice." (`g3.md:845`) sits above the
  table's last row (`g3.md:846`), which no longer renders. Move the row
  above the sentence.

The two adapted texts are otherwise consistent with the rest of v3. The
pin rule (`g3.md:758-767`) agrees with §3 step 3 (`g3.md:194-197`) and §17
item 2: no text still offers per-DLL strong roots as an escape. D0's
numbers contradict nothing; only the pilot file above needs changing.

#### 4. Open items, one line each

- S1: say that `Bc` decisions come from the port and that M1 makes no
  BCODESXP (before M1).
- D0: replace pilot `coerce.c` with `attrib.c`; measure `bind.c` on
  `unlist`/`cbind`/`rbind` (before D0 runs).
- S3: M0 needs four experiment switches on a `robigo-core` branch, and
  the adoption rule a same-mode baseline (before M0 closes).
- S2, S4, S5, S6, S7: one-line wording and table repairs.
- D1: Astra's `Out`-slot allocation variant is still neither listed nor
  ruled out.


### Opus

### Council round 5: `docs/g3.md` v3 (Claude Opus 5.5)

Base: worktree at `4cff0ceb` plus the uncommitted v3 (869 lines). Line
numbers are v3's. Nothing was written into the repo. One GNU R check ran
on the host R (4.6.1) from `/tmp`. `R450/` is
`~/.cache/robigo-research/rsrc/R-4.5.0/src`.

#### 1. Score: 9/10 (round 4: 8.5)

v3 fixes all four of my round-4 blocking items, and four of the five
non-blocking ones. Two edits introduced real contradictions (§3.1 and
§3.2 below). Each needs one or two sentences. M1 can start once §3.2 is
fixed, because it changes M1's scope. The rest can land in the same PR.

#### 2. Round-4 items

| item | status |
|---|---|
| B9 residue (M1 needs appendix rows; E6 needs `dyn.load`/`.Call`/`print`) | **fixed**: M1 row (685) takes the allocation, `collect_needing`, roots, trace, debug and removed rows, plus `dyn.load`, `.Call` by name and `print`/deparse for the E6 types. M2 (686) is the layout row only |
| N1 own-ALTREP "allocation-free" | **fixed**: §2.4 (104-110) uses my text and cites `ExpandDeferredStringElt`. §2.2 (71-72) has the cache exception |
| N2 per-DLL strong roots | **fixed**: §3 (194-197) is diagnostic-only, as are §17 item 2 (851-853) and §15.1 (761-762) |
| N3 base-package natives vs L4 | **fixed**: §10 (565-567) "all of them by M8"; M8 (692) "no C builtins, no base-package natives" |
| nb1 cmp.R decisions at the same moment | **fixed**, and stronger than my text (§9.1 473-481: BCODESXP made when GNU R's JIT would replace BODY, and observation never compiles). This edit causes §3.1 and §3.2 below |
| nb2 D0 bar written down | **fixed** (539-543: at least 150 builtins and at most 1.10x). One ambiguity remains: "a Rust port" does not say which. Smallest fix: "...the instructions per call of the same builtins ported to `robigo-rt`'s API for D0" |
| nb3 hook table for builtins below `rt` | **fixed** in §10 (568-571). But E6 (817-820), the exit that §10 points to, neither runs `lapply` or `[` dispatch nor records the cost. Append to E6: "; the hook table's cost per call on `lapply` and `[` dispatch is recorded" |
| nb4 tie rule | **fixed** (632-633: "a tie does not count as fastest") |
| nb5 L6 wording vs the symbol arena | **not fixed**. L6 (43) still says "no process-global mutable state", and §5.3 (305) cites L6 for a process-shared arena behind a lock. Append to L6: "; the one exception is A.2's process-shared symbol arena (append-only, interning under one lock, `docs/core.md` §10), §5.3" |

#### 3. What v3's edits broke

##### 3.1 Compiled state "belongs to the body" is wrong for a closure duplicated before compilation (§6.1 331-334, §9.1 473-477)

v3 says: "Whether a body is compiled, its frozen bindings and its BCODESXP
belong to the body, so a `duplicate`d closure shares them". §9.1 adds
that the BCODESXP is "stored with the body". Taken together, this hangs
the compiled state on the source LANGSXP. `duplicate` shares that
LANGSXP (`R450/main/duplicate.c:304`, `SET_BODY(t, BODY(s))`). So a copy
made *before* the JIT would look compiled as soon as the original is.

GNU R does something else. The JIT replaces the BODY slot of the one
closure it compiles (`eval.c:1908`, `SET_BODY(fun, BODY(val))`). A second
closure gets the same object only when its own JIT hits the cache
(`eval.c:1874`). The host oracle confirms this:

```r
f <- function(x) x + 1; g <- f; attr(g, "a") <- 1   # g duplicated
for (i in 1:3) f(i)
typeof(.Internal(bodyCode(f)))   # "bytecode"
typeof(.Internal(bodyCode(g)))   # "language"  <- v3 would say "bytecode"
for (i in 1:3) g(i)
identical(.Internal(bodyCode(f)), .Internal(bodyCode(g)))   # TRUE (JIT cache)
```

**Replacement for §9.1's "...and stored with the body":**

> ...and installed as the closure's BODY, as GNU R's `SET_BODY` does
> (`eval.c:1908`; the JIT cache hands the same object to a matching
> closure, `eval.c:1874`). Installation keeps robigo's compiled variants;
> §6.1's installed-BCODESXP path runs it from then on.

**In §6.1, replace "belong to the body, so":**

> belong to the BODY object (the BCODESXP once installed), so a closure
> duplicated after compilation shares them and one duplicated before does
> not;

##### 3.2 M1 now needs GNU R's compiler package, which it does not have (§9.1 473-478 vs the M1 row 685 and E5 813-816)

Under v3, crossing GNU R's JIT threshold runs GNU R's compiler package,
and the `Bc` profile's decisions "are cmp.R's, recorded at that same
moment". M1 has "the VM with both profiles", and E5 tests
`getBindingType` "after GNU R's JIT threshold". M1's base subset
(`calls.R`'s reach: `conditions.R`, `stop`, `structure`, ...) has no
compiler package. The compiler package is large R code that needs much
of base, and M3 is where base boots and where BCODESXP is listed. As
written, M1 cannot reach the `Bc` profile.

**Append to the M1 row:**

> ; `Bc` past the JIT threshold with cmp.R's decisions for M1's ops
> recorded from GNU R as fixtures (BCODESXP crystallization by running
> the compiler package is M3's)

##### 3.3 Small inconsistencies (one line each)

- **§16 table broken** (844-846): "Every mode in this table has a CI job
  on the `rt` slice." sits between the `ROBIGO_REF_EVAL=1` row and the
  `per-version oracle` row. The last row is cut off from the table. Move
  the sentence below the table.
- **PIN_REPORT's meaning**: §15.1 (762-765) now calls a hit "a candidate
  missing `PROTECT`" (a stale word looks the same). §3 (221) still says
  "(a missing `PROTECT` in that package)", and §16 (838) says "packages
  that rely on an unprotected C local". Change both to "candidate missing
  `PROTECT`s (confirmed with `ROBIGO_GC_STRONG_DLL`, §15.1)".
- **Appendix `roots` row vs §15.1**: the row says "the `.Call` stub,
  narrowing and zeroing removed", with no condition, and M1 builds it.
  §15.1 (765-766) says "removed only on their separate measurements".
  Change the row to "the `.Call` stub removed; narrowing and zeroing
  removed per §15.1's measurements".
- **§12 `inspect` row** (607): "a BCODESXP on observation" contradicts
  §9.1's "observation ... never runs the compiler". Change it to "the
  installed BCODESXP (§9.1)".


### Muse

### Council round 5 review of `docs/g3.md` v3 (Muse Wonder Full)

Scope per `brief-r5.md`: confirm my round-4 open items (B4, B5, B10, NB1,
and the five non-blocking items in `muse-r4.md` §4) against v3, and flag
contradictions v3's edits introduced. Read-only; no new scope. Citations
are `docs/g3.md:<line>` in the worktree.

#### 1. Score: 8/10

Round 4 promised 8 once the four one-paragraph repairs landed; they have
(B4, B5, NB1 verbatim or better, B10 in the adapted form the brief
discloses), and three of the five non-blocking items landed with them.
What keeps it at 8 and not higher is unchanged from round 4: two
non-blocking items are still open, and v3's merges left two wording
contradictions (§3 below). None of it blocks M1.

#### 2. Round-4 open items, one by one

- **B4 (slot-frame normative reference and deviations) — Fixed.** "`fable-r2.md` §3.1-3.2 in `docs/g3-council.md` (which lands with this doc; M0 does not close without it). Its semantics are normative; its word map is indicative … Deviations from it, exhaustive: the overflow area is the default …; listing order is per-binding definition stamps …; `R_varloc_t` stays GNU R's `{ SEXP cell }` …; the slot kinds add unbound and cell. Any other conflict with §3.1-3.2 is a bug in this doc" (`docs/g3.md:252-262`), and M0 now names the file (`docs/g3.md:684`); the semantics/word-map precedence split replaces my flat "§3.1-3.2 wins" and is coherent. As an artifact, `docs/g3-council.md` is still absent from the worktree (verified) — pending by the text's own terms (lands with the PR), now gated by M0.
- **B5 (BCODESXP produced on observation) — Fixed.** "A GNU R-shaped BCODESXP is crystallized at the moment GNU R's JIT would replace BODY, at a call boundary, by running GNU R's compiler package … in the compile environment, options and base bindings in force at that moment … Observation … only exposes the stored object and never runs the compiler; a closure GNU R has not yet compiled has no BCODESXP to expose" (`docs/g3.md:473-481`) — my text, strengthened by the §6.1 body/closure ownership split (`docs/g3.md:328-336`).
- **B10 (pin adoption rule) — Fixed, as adapted.** "Adoption rule, written before those runs: `robigo-heap` adopts pins only if no capi-corpus or package-suite case that passes today fails under pins and the premature-finalization probe passes; any failure reopens this decision (strong roots are not a per-package escape hatch …). … Narrowing and zeroing are removed only on their separate measurements, each leaving both probes unchanged" (`docs/g3.md:758-766`), with `ROBIGO_GC_STRONG_DLL` diagnostic only in §3 (`docs/g3.md:194-197`) and M0 gated on it (`docs/g3.md:684`). The per-DLL escape I proposed is gone per Opus N2; the stricter rule is better than mine and I accept the adaptation.
- **NB1 (restartable-builtin parenthetical) — Fixed.** "A whole builtin may be re-run only if no committed observable effect (store, warning, callback, RNG draw, native call) occurred before the particular failed allocation; having no effect before the first allocation does not establish restartability after later ones" (`docs/g3.md:164-167`) — my fix, and the second clause names the original fallacy, which is more durable than my wording.
- **Non-blocking 1 (§5.2 "bit for bit") — Fixed.** Hashed environments now "keep GNU R's logical structure exactly (FRAME pairlist of binding cells, HASHTAB chains, `HASHPRI` fill count)" (`docs/g3.md:297-299`).
- **Non-blocking 2 (eval-count poll unit) — Not fixed.** V3 text: "`eval.c:1096`, the eval-count poll, whose unit robigo counts as GNU R does; `eval.c:6247`, the byte-code poll" (`docs/g3.md:174-178`) — "as GNU R does" still does not name the unit in a VM with no per-node `eval()`. Smallest replacement for the parenthetical: "(unit: one per `eval()`-equivalent node evaluation under `Eval`, polled when the count exceeds 1000, `eval.c:1091-1099`; one per `BC_CHECK_SIGINT` site under `Bc`, polled past 1023, `eval.c:6251-6257`)". Non-blocking until M3, as in round 4.
- **Non-blocking 3 (§17 item 4 overstates §12) — Not fixed.** V3 still claims "§12 extended with the round-3 rows" (`docs/g3.md:856`), but §12 (`docs/g3.md:604-625`) still has no row for interrupts as an observable (non-catchable jump, timing against output), `fork`/`mclapply` against the value-stack reservation and shared symbol arena, or S4 dispatch/`methods` invalidation semantics (§13 measures S4 speed only, `docs/g3.md:654`). Smallest replacement: either add those three rows to §12 or change `docs/g3.md:856` to "§12 extended with the round-3 rows except interrupts, `fork`/`mclapply` and S4 invalidation, which are added before M3."
- **Non-blocking 4 (`ROBIGO_REF_EVAL=1` CI job) — Fixed.** "A naive reference interpreter … as a differential oracle with its own CI job on the `rt` slice" (`docs/g3.md:482-484`), plus "Every mode in this table has a CI job on the `rt` slice" (`docs/g3.md:845`) — the latter sentence's placement breaks the §16 table; see §3.
- **Non-blocking 5 (M7 flip and delete in one PR) — Fixed in M7, contradicted elsewhere.** M7 now reads "**flip**: `robigo-rt` becomes the default in one PR; a follow-up PR deletes `robigo-runtime`, `robigo-core`, `robigo-capi`, the bridge and `rlib/` (so the riskiest step stays bisectable)" (`docs/g3.md:691`) — but see contradiction 1 in §3.

#### 3. Contradictions v3's edits introduced

1. **Flip-vs-delete timing (v3's fix of non-blocking 5 vs lead decision L1 and §14.1).** M7 splits flip and deletion into two PRs (`docs/g3.md:691`), but L1 still says "the flip deletes the old runtime, the bridge, `robigo-core` and `robigo-capi`" (`docs/g3.md:38`) and §14.1 still says the three crates are "deleted at the flip" (`docs/g3.md:675-677`). L1 is a lead decision, so the two-PR split needs the lead's nod or L1/§14.1 amended to "deleted in a follow-up once the flip is green"; as written, an implementer can cite either. Wording only; does not block M1.
2. **§12's inspect row vs v3's §9.1 crystallization rule.** §12 still describes the BCODESXP as "a BCODESXP on observation" (`docs/g3.md:607`), which is the v2 produce-on-observation model that §9.1 now forbids ("Observation … only exposes the stored object and never runs the compiler; a closure GNU R has not yet compiled has no BCODESXP to expose", `docs/g3.md:478-481`). Smallest replacement in the row: "the stored BCODESXP (§9.1), where GNU R's JIT has made one".
3. **§16's table is broken by the inserted sentence (minor, formatting).** "Every mode in this table has a CI job on the `rt` slice." now sits between the table's rows, orphaning the last row as "|| per-version oracle | behaviour against GNU R 4.6.2 |" (`docs/g3.md:843-846`). Move the sentence below the `per-version oracle` row. (While there: "per-version oracle" is arguably not a mode; the sentence's claim that every row has a CI job is only clean if that row moves out of the table.)

#### Closing

Two non-blocking items remain open (eval-count unit, three §12 rows), both
M3-facing and both one-line fixes; two wording contradictions to reconcile
in the PR that lands the doc (L1 vs M7, §12 vs §9.1). The four round-4
blocking items are closed. 8/10; no further round needed from me once
§3 items 1–2 are reconciled, and M1 is not blocked by either.


## Experiment A results

### Experiment A: results so far (coordinator, 2026-11-13)

Tree: main `4cff0ceb` plus two uncommitted switches:
`ROBIGO_GC_PIN_ONLY=1` (`crates/robigo-core/src/gc.rs` `collect_needing`:
`process_weak_refs` before the conservative stack scan, so stack words pin
memory but decide no finalizer) and `ROBIGO_CALL65_PLAIN=1`
(`crates/robigo-core/src/shim.c` `robigo_try_call65`: call the routine
directly, without the stub's scan narrowing and 1 KiB zeroing). Release
build (`scripts/build.sh`).

1. **Planted stale words (the ledger's proof test).** capi
   `native_call_frames_keep_nothing` plants a pointer to a dead external
   pointer in every word from the routine's frame to the bridge's entry,
   then collects inside a `.Call`, and requires the finalizer to run.
   - stub off, normal order: **FAILS** ("a stale word in the frames of a
     native call kept an object alive") — the test discriminates.
   - stub off, pin-only: **passes**.
   - `cargo test --release -p robigo-core -p robigo-capi` with both
     switches: all 65 pass, including `arguments_survive_the_routine` and
     core `stack_parameters_of_a_routine_are_roots` (memory safety: objects
     held only by C stack words survive).
2. **Stress script** (`.Call('pointer', id)` at R depth a, C churn or
   `gc()` at depth b, a, b in 0..24, finalizer checked per id), all tiers:
   one run per cell: baseline 0 misses; pin-only 0; pin-only + stub off 0;
   stub off with normal order 1 miss ((20,19), interpreter, churn).
   30-run repeat (churn, depth 24, ASLR on), misses in 30x625 per tier
   (interp / VM / VM+JIT): baseline 0/0/0; pin-only 0/0/0; stub off,
   normal order **30/0/0** (one miss in every interpreter run);
   pin-only + stub off 0/0/0.
3. **Cost.** Instructions per no-op 3-argument `.Call` (callgrind, 40,000
   minus 20,000 calls): stub 7,631; plain 7,511 (−120, −1.6%); pin-only +
   plain 7,513. Pin-only adds no per-call work.
4. Conformance suite (`cargo test --release -p robigo-cli --test
   conformance`, all tiers) with both switches: 601/623, one ratcheted
   regression (`graphics/cairo-devices.R`), which fails identically without
   the switches: it needs grDevices natives, which the local run disables
   with `ROBIGO_NATIVES=/nonexistent` (CI builds them). The tier-identity
   tests (`compiled_matches_interpreted`) pass. No regression from the
   experiment.

Semantics changed by pin-only: an object reachable *only* through C stack
words (a package that forgot to PROTECT it) has its weak references and
finalizer processed at that collection while its memory stays allocated.
GNU R would free such an object outright.
