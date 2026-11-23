## 2026-11-22: C1: the translation cache

robigo-rt translates the byte code in its lazy-load databases (V3) at a
closure's first call and a promise's first force. GNU R decodes the same byte
code at unserialize (`R_bcEncode`) and pays nothing at the call. With the JIT
on, the compiler's load made robigo-rt's start 95.24M instructions
(`--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`, rdb boot, main 40f4b6f71) against
GNU R's 87.63M on X's like-for-like table. Translation alone was 15.6M of the
difference ([[c1-translation-split]]).

The cache keeps those translations, written once when the tree is built and
read where a translation would be made. A start never writes it.

### What a start costs

`--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`, rdb boot. Each binary is
measured with and without its cache. Main 24f3fd36a has the dense tables
(#974) and the crosscheck report fix (#973).

| | without the cache (`ROBIGO_TRANSLATION_CACHE=0`) | with it |
|---|---:|---:|
| JIT on, instructions (callgrind) | 92.84M | 82.07M |
| JIT off, instructions | 50.21M | 46.80M |
| JIT on, wall (median of 100) | 27.68 ms | 25.45 ms |
| JIT off, wall | 20.26 ms | 19.62 ms |
| JIT on, cycles | 79.9M | 72.4M |
| JIT on, system time | 8.2 ms | 8.1 ms |

- **Against GNU R:** GNU R's start is 87.63M (X's like-for-like table).
  With the cache the JIT-on start is 5.6M below it, which meets L5's
  startup condition for this part. On main 40f4b6f71, without the dense
  tables, it was 95.24M → 81.31M.
- **Wall time** was taken with `hyperfine`, 100 runs each, at a load of
  22–23 on the 24-CPU box.
- **Reads:** the file is mapped with `MADV_RANDOM` (below, "The start
  rows in CI"). Before that, entries were read: first each with its own
  `pread`, about 429 a start, which took 3.1 ms of system time on this
  WSL2 kernel; then in 32 KB chunks laid out in translation order, 8 reads
  a start.
- **Hits:** all 429 translations of that start hit
  (`ROBIGO_TRANSLATION_CACHE_STATS=1`).
- **Equality with fresh translations:** the verify pass on the merged head
  compares all 1,731 entries with the dense-table translator's fresh
  translations, 0 differing. Before #974 merged, its translator wrote
  byte-identical entries for all 1,731.
- **rt-ci:** the full run passes on the branch's head, the rdb crosscheck
  comparing 97,776 closures. Two new steps go in the `modes` shard:
  - the rdb boot's conformance without the cache, 0.18 minutes here;
  - the cache as damaged input (`scripts/rt-tcache-mutate.py`, 300
    corrupted files of each package), 0.38 minutes here.

  The shard's budget is 13 minutes, up from main's 11, and its job's
  timeout is 26. The basis is #959's run 45072863916, a slow runner, at
  9.15 minutes for main's steps. The two steps are scaled to that run by
  1.6: on the five conformance steps that both it and the local run made,
  the run took 1.34 minutes against 0.84 here. That makes 0.29 and 0.61
  minutes, 10.05 in all, plus 20%, rounded up.
- **The cargo-fuzz target** (`fuzz/fuzz_targets/rtc.rs`) runs locally:
  8×60 s, about 60k inputs, no finding. CI's fuzz job runs
  `scripts/fuzz.py` against the old runtime's binary, not cargo-fuzz
  targets, so a job for this target is A1's to add.

- **CI's startup rows (#978):** `rdb-jit` and `rdb-nojit` are the default
  starts, so with this PR they run with the cache. Against main's binary
  they show about −11% and −7% instructions, which passes, because the
  limits are upper bounds. Bar 5 is judged on `rdb-jit` against GNU R's
  JIT-on start in the same CI run, not on local numbers: on the runner, GNU
  R counts 84.14M and robigo-rt's uncached start 95.67M. The reserved
  `rdb-cache` became `rdb-nocache` (JIT on, `ROBIGO_TRANSLATION_CACHE=0`),
  which guards the translator's own cost like for like with main, whose
  binary ignores the variable.

### The start rows in CI, and what changed for them

The first CI run of the startup gate (#978) on this PR's b876d8750 failed
two rows. Bar 5 held there: `rdb-jit` 84,070,737 instructions against GNU
R's 84,143,909 on the same runner.

**The interim start, +80,848 instructions (+0.52%), though the interim
boot never reads the cache.**
- **What moved it:** the PR grew `RtState` by 248 bytes, its writer's
  capture held inline (824 to 1,072 bytes). A start boxes five. The calls
  stay the same: 2,271 `realloc` and 2,896 `malloc`, with the same callers
  on both binaries (callgrind). Their cost in glibc changes, +12k and +15k:
  the same work, priced by the heap's layout.
- **Found with:** an `LD_PRELOAD` shim that logs every allocation's size,
  diffed between the two starts. Their first difference was this box.
- **The mechanism** (H, on #984's same row, from CI's binaries): a start's
  count is bimodal, in steps of about 80k, and the heap's layout picks the
  step; R_HOME's path length alone moves it. Across a step the difference
  is +60k in `memcpy` and +10k in `_int_malloc`, under `realloc` from
  `Wide::parse`'s growing tables (base.rs: 1,193 names and a HashMap
  grown by doubling). Each reallocation extends in place or copies,
  depending on what lies above it. `RtState`'s size is one lever on that
  layout, not a size-class cliff. #984 sizes those tables up front.
- **What this PR does:** `RtState::tcache` is one word, a `OnceCell` of a
  box that only the writer and a verification make. `RtState::base_src`
  becomes a `OnceCell<PathBuf>`, 8 bytes smaller, set once by the boot's
  load, so `RtState` keeps main's size with the cache's word. Without that,
  on main 17a3d68ca the one word alone gave the interim start +0.68%
  (15,372,768 against 15,268,258).

**Against main 114113d6e** (#984 on main, Wide's tables sized up front):
scripts/rt-startup.py, each binary from its own packed trees at one path,
5 runs; minor faults the median of 7 starts; wall time from hyperfine, 60
runs at a load of about 41:

| start | main 114113d6e | this PR | change |
|---|---:|---:|---:|
| interim, instructions | 14,889,275 | 14,881,633 | −0.05% |
| rdb-jit, instructions | 92,528,723 | 81,446,941 | −11.98% |
| rdb-nojit, instructions | 50,063,475 | 46,412,759 | −7.29% |
| rdb-nocache, instructions | 92,730,345 | 92,881,119 | +0.16% |
| rdb-jit, anonymous peak | 6,928 KiB | 6,776 KiB | −152 KiB |
| rdb-jit, peak RSS | 15,764 KiB | 16,348 KiB | +584 KiB (+3.7%) |
| rdb-nojit, anonymous peak | 4,676 KiB | 4,656 KiB | −20 KiB |
| rdb-nojit, peak RSS | 13,512 KiB | 13,960 KiB | +448 KiB (+3.3%) |
| rdb-jit, minor faults | 2,106 | 1,898 | |
| rdb-nojit, minor faults | 1,324 | 1,296 | |
| rdb-jit, wall | 45.4 ms | 43.2 ms | −4.8% |
| rdb-nojit, wall | 31.0 ms | 29.8 ms | −3.9% |

- **Peak RSS:** the growth is the `.rtc` files' pages, mapped and file-backed.
- **`rdb-nocache`:** +0.16% is what the off-switch costs (the record's key
  and the checks before a miss).
- **The `OnceCell` on this main:** it does not help the interim row. With
  `base_src` a `RefCell` again, the start measured 14,691,939, a heap-layout
  step below. It stays as it is.

**The fallbacks** (rdb-jit from one tree of this PR, callgrind):
- cache off: 92,782,476 instructions;
- absent: 92,712,966;
- stale (another build ID): 92,774,273;
- unreadable (a directory at each `.rtc`): 93,052,158;
- zero-length: 92,914,464;
- cut short of its table: 92,913,399.

Every one has 0 hits and says nothing, and none maps a file it cannot use.
The unreadable variant's +0.29% is in glibc (`free`, `malloc`'s paths), not
in work. The zero-length and short ones pay about 62 instructions a fetch
(27k a start) to rule out a database that did not open.

**`rdb-jit`'s anonymous peak, +176 KiB against a limit of +68.** It was
the header and entry table read whole, about 62 KiB, and the kept read
chunks, 128 KiB. The file is now mapped read-only with `MADV_RANDOM`: a
page comes in when it is read, alone, as file-backed memory, with no
anonymous copy at all. Measured with scripts/rt-startup.py, each binary
from its own packed trees:

| | main dc256de9f | chunks: 2 × 32 KB kept | mapped (`MADV_RANDOM`) |
|---|---:|---:|---:|
| rdb-jit instructions | 92.59M | 82.17M | **81.72M** |
| rdb-jit anonymous peak | 6,840 KiB | 6,916 KiB (+76) | **6,788 KiB (−52)** |
| rdb-jit peak RSS | 15,388 KiB | 15,468 KiB | 15,848 KiB (+3.0%) |
| rdb-jit minor faults | 2,114 | 1,930 | 1,891 |
| rdb-nojit instructions | 50.21M | 46.81M | **46.68M** |
| rdb-nojit anonymous peak | 4,660 KiB | 4,680 KiB | **4,620 KiB (−40)** |
| rdb-nojit peak RSS | 13,176 KiB | 13,356 KiB | 13,532 KiB (+2.7%) |
| rdb-nojit minor faults | 1,332 | 1,314 | 1,291 |

- **What the mapping gives:** the anonymous peak is below main's, because
  a hit's code costs less than a translation's working memory. The file's
  pages are in peak RSS instead, which the gate limits at +5% for the
  startup rows, as file-backed memory. It saves 0.45M instructions over
  the chunks, the copies out of them.
- **The kept chunks before the mapping** (one binary and its own trees per
  setting; reads by strace): 4 × 32 KB, 6,988 KiB, 8 reads JIT on and 2
  off; 2 × 32 KB, 6,924 KiB, the same reads; 1 × 32 KB, 6,924 KiB, the
  same reads, 0.03M more instructions; 4 × 16 KB, 6,924 KiB, 13 and 4
  reads.
- **What the mapping costs:** a cache file truncated in place while a
  process maps it would make a read past its new end fault (SIGBUS),
  where a `pread` would have been a miss. The tree's files are never
  written in place. `scripts/rt-home-db.sh` builds a new tree and moves it
  into place, so a running process keeps the old file.
  `scripts/rt-tcache-mutate.py` rewrites the copy's cache only between
  starts.
- **Path dependence:** the anonymous peak moves by about ±24 KiB with the
  tree's path, from one binary (7,012 KiB in this checkout's tree, 6,988
  KiB in rt-startup's scratch tree). CI's gate compares both binaries from
  trees at one path on one runner, so it does not see that, but a local
  measurement does.

**The verify protocol** (the bot's round 2): a verification's fresh
translation that stopped with a retryable `Need` (`Gc`, `Force`) was still
compared. Its capture was empty, so it counted as "differing", and the
attempt then ran again and verified again. Now `tcache::verify_abort`
resets the state, compares nothing, and counts the stop ("verify retried"
in the stats). `tests/boot_db.rs`'s
`a_verification_stopped_for_a_retry_compares_nothing` makes such a stop
certain: under `ROBIGO_RETRY_TORTURE` every fresh translation of a
verification stops once inside it. rt-ci's feature-builds shard runs it,
with base's tree written by that build first. "1,731 entries, 0
differing" held before only because no stop fell inside a verification
in those runs: 25 starts with one refused allocation each
(`ROBIGO_FAIL_ALLOC_AT`, spread over the 309k a verify start makes) put
none inside one.

### The design (the page the foreman approved, with its five amendments)

**Where it lives.** Beside R_HOME, never inside it: `<R_HOME>-cache/<pkg>.rtc`.
- An installed tree's R_HOME is `<prefix>/lib/robigo`, so its cache is
  `<prefix>/lib/robigo-cache`. This checkout's tree has
  `.robigo/rt-home-db-cache`.
- Nothing a user sets points the reader at another file; the tree is
  chosen as R_HOME is. `ROBIGO_TRANSLATION_CACHE` is `0` (off), `verify`, or
  unset (on). The writer (`ROBIGO_TRANSLATION_CACHE_WRITE=1`, rt-home-db.sh)
  takes the place and the stamp from the tree it builds.
- `R.home()`, `list.files()` and `system.file()` see nothing new, and the
  `.rdb` files stay GNU R's bytes.
- `scripts/rt-home-db.sh` writes the cache with the tree's own binary. That
  is every closure of base.rdb and compiler.rdb, its nested function
  literals, and the promise codes that start forces. Then it runs the verify
  pass below; a difference fails the tree.
- About 2.1 MB in all (base 1,568 entries, compiler 163). The build adds
  about a second and a half to the tree's 15 s (16 s without, 17 s with).
- **Coverage:** the tree builds base and compiler as databases, and both
  have a cache. tools stays source in robigo-rt's tree (rt-home-db.sh does
  not run tools-mklazycomp), so it has no byte code to cache, and the
  tree builds no other package. The writer is generic over a package's
  `.rdb`: each package L's tree build adds as a database gets its `.rtc`
  from the same step, with no change here.
- **Installed packages** have no cache yet. The hook's place is the install
  step that writes a package's `.rdb`: `tools:::makeLazyLoading`, called by
  `R CMD INSTALL`'s lazy-load step, which `lazyLoadDBinsertValue` serves.
  A writer run there, with the tree's binary, would write `<pkg>.rtc` beside
  that library's package by the same format and key.

**The key.** A record is fetched by `lazyLoadDBfetch(c(offset, length), file)`.
- When `file` is `<R_HOME>/library/<pkg>/R/<pkg>.rdb` of the running R_HOME
  and that package's cache matched, the record's reader numbers the
  BCODESXPs it makes, in order.
- It marks one that has an entry with a 24-byte RAWSXP stub in its fourth
  word (`aux`), naming the database, the record's offset, the ordinal and
  a checksum of the record's stored bytes.
- A promise code is keyed with a hash of its forcing frame's shape too.
- The key context belongs to that one reader. Anything a refhook
  unserializes is read with none, so other byte code is never mistaken for
  the database's.

**The `aux` word, one layout** (for this, and for P3 step 2's inliner): it
holds R_NilValue, a code object (the translation), or a stub.
- Every reader tests for a code object first (`code::cached`). A stub reads
  as "not translated yet" everywhere, and only `translate_code`'s miss path
  looks it up.
- A hit is materialized there, then `cache_on` replaces the stub, as it does
  for a translation.
- All three are traced heap objects, so the collector and the freeze need
  nothing new.

**What the checks guarantee, and what they do not.**
- **Against damage**, meaning any bytes that do not carry a valid keyed
  checksum, an entry is a miss, and the code is translated as with no
  cache. `scripts/rt-tcache-mutate.py` and the fuzz target are the proof.
- **Against a forged entry** with a valid checksum there is no guarantee. A
  forged entry can break the VM's register-state invariants, which the
  verifier does not check: the rehashed mutation run hit `vm.rs:2118`'s
  `unwrap` on a `for` state register, and per Sol's review it can read out
  of bounds.
- **Why that is acceptable for now:** whoever can write the `.rtc` can write
  base.rdb, and base.rdb already gives arbitrary code execution (R code that
  calls `dyn.load` or `system`). So a forged `.rtc` grants nothing new.
- **The trust is in the types:** materializing goes through the `unsafe`
  constructor `code::Verified::from_cached_translation`, whose caller
  promises that the code is this build's translator's own and intact.
  Its one call site names the basis: build ID, stamp, keyed checksum and the
  typed check. `Verified::new`'s documentation says what it does not check.
- **Hostile byte code without the cache** reaches the same class of states,
  through the translator itself (a mutated BCODESXP from `mkCode`/`bcClose`
  or a crafted RDS): issue #976, the verifier and translator work (A).

The checks, in order:
- A file is ignored whole unless its header's build ID is the running
  binary's (read from `/proc/self/exe`'s GNU note) and its stamp is the
  tree's `.rt-home-db`. Nothing is hashed at start.
- An entry applies only to the record it was written from. It carries a
  checksum of that record's stored (compressed) bytes, and the reader
  hashes the record it just read. A mismatch is a miss (`StaleRecord`):
  a `.rdb` replaced under a valid stamp, with a record at the same offset,
  gets fresh translations. This costs 0.063M instructions a start (L's P3;
  a hash of the decompressed bytes cost 0.45M).
- Each entry carries a checksum, so damage makes it a miss. Without one, a
  damaged entry that still passes the checks could run, and the mutation test
  found one that looped forever.
- An entry is decoded field by field; an operation goes through
  `Op::from_u8`, never reinterpreted bytes.
- Its constants are resolved by reference: a checked path from the
  BCODESXP's own pool, a symbol, an immortal, a primitive by table index, a
  CHARSXP, a plain vector, or a call or list rebuilt from references.
- Then it passes a typed check (`tcache::typed_ok`), then
  `code::Verified::new`.

**The typed check.** The verifier's guarantees were written for a trusted
producer: robigo-rt's own translator, which gets a constant's type right by
construction. The cache makes the code-object format a boundary with
untrusted input, so the loader adds what a trusted producer never needed:
field-by-field decoding, and a check of each operand's constant against the
type the VM reads it as. An audit of `vm.rs` and the code it calls
established what each operation reads unchecked:
- symbols in GetVar, SetVar, Guard, GetFun, Missing, ForPrep and ForStep;
- calls walked raw in Call, CallCodes and BaseGuard;
- a CHARSXP in ErrorCall;
- primitives by their table word;
- BcAssign's plan layout.

**Compared, not trusted:**
- Under `ROBIGO_TRANSLATION_CACHE=verify`, and whenever the crosscheck runs
  (`ROBIGO_BC_CROSSCHECK`), every hit is also translated fresh, with no cache.
  Both are compared as entries: references relative to the same BCODESXP, so
  they are equal exactly when the translations are.
- A difference is a crosscheck "unexpected" finding, and the fresh code runs.
- `rt-home-db.sh`'s pass verifies all 1,731 entries, and fails the tree on a
  difference.
- rt-ci runs the rdb boot's conformance with the cache and without it.

### Tests

- **Fallbacks:** the start when the cache is absent, stale (another build
  ID) or unreadable costs what a start with `ROBIGO_TRANSLATION_CACHE=0`
  costs (95.39M, 95.61M and 95.39M against 95.50M, JIT on), with its output
  and no message: every lookup is a miss.
- **Peak RSS** (scripts/rss.py, three runs, rdb boot): binarytrees 28,332
  KiB with the cache against 28,264 for the base binary, listgrow 52,140
  against 52,104, churn 85,956 against 85,744, with the entries read in
  32 KB chunks then (the mapped file's numbers are above).
- **The boot freeze (H):** on the head with #959 merged, the rdb boot's
  conformance with the cache ran under the freeze's check. With
  `ROBIGO_GC_FREEZE=1 ROBIGO_GC_FREEZE_CHECK=1` it ran three times; with
  gctorture and `ROBIGO_GC_FREEZE_CHECK=16` once. The check never failed:
  every report was empty. A start under the freeze takes 430 hits.
  The only failures were cases timing out at a load of 55-65:
  - `stress/finalizer-depths.R` took 25 s under the every-collection check
    against the runner's 20 s, both with the cache and with
    `ROBIGO_TRANSLATION_CACHE=0`;
  - under gctorture, `lang/tailcall-frames.R` and
    `memory/finalizers-self-ref-envs.R` hit the 600 s limit. The latter is
    one of the nightly freeze check's slow cases, which get 4,400 s there.

  A stub is a RAWSXP marked
  `extra::vector::NO_FREEZE` (`robigo-heap` layout/defs.rs). The freeze
  leaves it ordinary, so its replacement, through `set_aux`, the barrier,
  writes over no frozen object. This half is here; H's pass PR adds the
  test to the freeze's walk, its half of the pairing.
- **Format:** `tcache::format` unit tests check that an entry and a file
  round-trip, that every truncation of an entry is a miss, that every
  single-bit flip of it is a miss or another well-formed entry, never a
  panic, and that a damaged entry is a miss. `fuzz/fuzz_targets/rtc.rs` runs
  the reader over arbitrary bytes (cargo-fuzz).
- **`scripts/rt-tcache-mutate.py`**, the runtime half. Damaged files: 300 of
  base.rtc and 1,500 of compiler.rtc, every start's output the same. Damaged
  entries with their checksums fixed up, so that the entry's own checks are
  what stop them: 400 of each. They gave 789 runs with the same output, 10
  R errors and one well-formed loop that never ended, and no crash or panic.
- **Conformance, rdb boot:** 704 of 710 with the cache and without it, case
  for case. The six are the known unratcheted ones.
- **Collectors:** with gctorture, with gctorture and `GC_MOVE=young:16`, and
  with `GC_MOVE=1`, each start's 435 hits materialize. The verify pass under
  gctorture with `young:16` verifies all 1,731 entries, none differing.

### Not yet, or not at all

- **A forged entry** carries no guarantee at all. An entry rebuilt with a
  matching checksum can break the VM's register-state invariants: it can
  panic, as `vm.rs:2118` did in the rehashed mutation run, or read out of
  bounds. Short of that, it can run well-formed code that is wrong, or that
  never ends (one in 400 did). This is acceptable for the reason given
  under "What the checks guarantee": the tree is one artifact, and whoever
  can write its `.rtc` can write its base.rdb. `ROBIGO_TRANSLATION_CACHE=verify`
  is the full comparison, and #976 is the verifier work.
- **A `.rdb` replaced by hand** under a valid stamp is detected record by
  record: an entry whose record's stored bytes differ is a miss. Nothing
  is checked at start, and the file as a whole is not.
- **Installing (L):** L5 installs and replaces R_HOME and its `-cache`
  sibling as a pair. The stamp covers everything that determines the
  `.rdb` bytes, so a tree rebuilt from other sources has a new stamp, and
  the old cache is ignored as a whole.
- **tools** has no cache while it stays source in robigo-rt's tree. It gains
  one with no change here when L's "tools from tools.rdb" item lands.
- **Promise codes** are cached for the frame shapes that the writer's start
  forces. Other shapes translate as now.
- **A user's package** has no cache. One written at `R CMD INSTALL` time could
  use the same format and key; that is out of scope.
