## 2026-11-17: unique.c's hash table on robigo's heap (#752's perf-rt failure)

- Why: L's #752 (L1) failed perf-rt on e7-unique (+0.55%) and e7-match
  (+0.33%), rows L1 does not touch. Callgrind per function, the merge
  commit `c2dded2f` against its first parent `518f77c1`:
  - e7-unique's extra instructions were all in glibc's malloc internals
    and a `memcpy`. robigo-rt's functions counted the same, and the number
    of calls into malloc, calloc, realloc and free was unchanged.
  - e7-match's were in robigo-rt's own probing: `Table::is_dup` +252,
    `lookup` −70.

  Two local variants showed the causes:
  - With base's boot lists, e7-match came back to base: L1 binds 17 more
    base closures at boot, which moves the CHARSXPs that `cshash` hashes
    by address.
  - e7-unique moved whatever startup did (+0.55% with base's boot lists,
    +0.24% without L1's startup tables). Its hash table was a Rust `Vec`
    malloc'ed on every call, so its cost followed glibc's free lists.
- What (foreman's fix 1(a)): port `HashTableSetup` as GNU R does.
  - `d->HashTable` is `allocVector(INTSXP, M)` on robigo's heap, every
    slot NIL, made and read in one `&Cx` interval (GNU R's `PROTECT`:
    nothing collects there). robigo-rtcore's
    `FreshVec::into_mut_slice` gives such an unpublished vector's
    elements for the interval.
  - `duplicated()`'s flags are the LGLSXP it returns, as `duplicated3`
    makes it; the forward case writes each flag as the vector is made,
    with no clearing.
  - `unique`'s copy loops over the flags, as `do_duplicated` does.
  - `anyDuplicated` lost its boxed iterator.

  These per-call mallocs are gone.
- The fill: LLVM turned the NIL fill (a known all-ones word) into a
  `memset` call, which callgrind counted at about 8,200 instructions per
  e7-unique iteration on main too (main's `vec![NIL; m]`). The fill value
  is opaque to the optimizer (`black_box`, once per table), so the fill is
  vectorized stores.
- Measured (this box, release, each binary from its own checkout and a
  fresh private target dir, deleted after; sha256/build id: main `fb429442`
  94a6e0c0…/f2a8732b…, branch `189ae056` d7f0cc99…/488dc099…, #752's head
  rebased on the branch `37ef7d27` 54ce5a18…/599263c8…):
  - perf-rt against main: e7-unique 126,475.8 → 123,004.9 (−2.74%),
    e7-match 99,038.9 → 92,019.0 (−7.09%); every other row within ±0.01%;
    peak RSS within ±0.4%.
  - `benchmarks/rt/layout-rows.toml` (#753's, five paddings), the spread
    across paddings:

    | row | main | branch |
    |---|---|---|
    | e7-unique | 126,352–126,492 (140, 0.11%) | 123,003–123,013 (10, 0.008%) |
    | e7-match | 98,903–99,029 (126, 0.13%) | 91,877–92,018 (141, 0.15%) |

    e7-unique no longer depends on the allocator's state. e7-match still
    moves with the keys' addresses (`cshash`): fix 2, hashing cached
    strings by content, is Max's decision.
  - #752's head rebased on the branch, against the branch: 0 failures
    (e7-unique +0.00%, e7-match +0.19%, s3 +0.09%, superassign −0.18%).
- Known limit, as before for the results: a table of 1 MiB or more (from
  131,072 keys) is a large `&Cx` request, counted while a collection is due
  and fatal under `ROBIGO_GC_STATS=check` (no rt case comes near it); the
  results of that size already were.
- Tests: `tests/rt/unique/hashtable.R`, recorded on 4.6.2 and ratcheted. It
  covers the fixed logical and raw tables, `fromLast` with `incomparables`,
  `anyDuplicated`, a 600-string table, the list scalar `match`, and
  `nmax`'s "hash table is full".
