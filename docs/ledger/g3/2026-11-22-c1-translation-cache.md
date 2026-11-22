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
- **Reads:** the first reader read each entry with its own `pread`, about
  429 a start. On this WSL2 kernel those took 3.1 ms of system time, which
  ate most of the wall-time gain. Entries are now laid out in translation
  order and read in 32 KB chunks: 8 reads a start, and system time is
  unchanged by the cache.
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
  against 52,104, churn 85,956 against 85,744. Entries are read by `pread`
  in 32 KB chunks, a start's own laid out together by the writer: 8 reads
  per start, about 60 KB of header and table resident.
- **The boot freeze (H):** the freeze check (`ROBIGO_GC_FREEZE=1
  ROBIGO_GC_FREEZE_CHECK=1`) runs once #959 merges: still to do. A stub is a RAWSXP marked
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
