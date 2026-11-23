# Startup rows in perf-rt, and the heap-shift median for allocating rows (2026-11-23)

X/P1, the foreman's request after the startup table (2026-11-21).

**Why.** Nothing in CI measured a start: perf-rt's rows are N-against-2N
differences, so a constant added to every start cancels out. H's boot freeze
(#959) was green on every gate as first pushed and cost +53–65% at every
start. The harness (`scripts/perf.py`, `scripts/idiom-vs-r.py`) is A1's; A1
read the plan first.

## The startup rows

- **The starts:** `scripts/rt-startup.py` runs `--vanilla -e 1` with
  `R_DEFAULT_PACKAGES=NULL` (and the oracle locale).
  - robigo-rt's starts are `interim`, `rdb-jit` and `rdb-nojit`
    (`ROBIGO_RT_BOOT=rdb`, `R_ENABLE_JIT=0`).
  - `rdb-pkgs` (L5's default packages) and `rdb-cache` (C1's translation
    cache) are reserved names, refused until defined.
  - `benchmarks/rt/m1-bars.toml`'s `[[startup]]` lists the starts the gate
    measures, each with the GNU R start shown beside it.
- **The trees:** each binary boots from the R_HOME trees its own build
  writes, as it ships. `perf.py rt-bars` builds `.robigo/rt-home` and
  `.robigo/rt-home-db` (`scripts/rt-home-db.sh --robigo-rt BIN`) for the new
  binary, measures it, then does the same for main's. A1 confirmed that one
  tree shared by two binaries is wrong: a rebuild for the other binary moves
  base.rdb's offsets. Building both trees takes about 32 s per binary.
- **The trees in CI.** perf-rt runs in the oracle image, which has no Rust
  toolchain, and `rt-home.sh` builds with cargo (the first run failed so:
  "cargo: command not found"). So the trees are build products:
  - `perf-rt-build`, which has the toolchain and now GNU R's sources, packs
    each binary's trees (`rt-startup.py --pack-trees`): the PR's in its
    checkout, main's in main's, by this checkout's rt-startup.py (main may
    not have it);
  - they ship as `dist/rt-trees.tar` and `rt-trees-main.tar`, about 20 MB
    each, and perf-rt unpacks each one before measuring that binary
    (`--trees`, `--main-trees`);
  - in CI `rt-startup.py` refuses to build trees.
- **Whether a tree travels:**
  - boot checks only that the tree has base (`has_base`, `has_base_db`);
  - the only absolute paths inside a tree are two informational records
    (`.rt-home-db-builder`, `.rt-home-db-built-at`), which nothing reads at
    boot;
  - both jobs check out to the same workspace path, and the two images share
    their Debian trixie base, so `tools.so` links the same;
  - checked locally: main's trees, packed in one checkout and unpacked into
    a bare directory, boot all three starts;
  - the pack takes every `.robigo/rt-home*` directory, so C1's translation
    cache (`<R_HOME>-cache`) will travel with its tree for `rdb-cache`.
- **A start's count follows its tree's path:** from that bare directory,
  whose path is longer, rdb with the JIT off counted 51.34M where the
  checkout gave 50.83M (+1.0%), JIT on +0.3%, interim +0.05%. Both binaries
  are measured from the same checkout path, so the gate compares like with
  like; GNU R's numbers beside them are from another path, report only.
- **Three numbers per start:**
  - **instructions:** callgrind's total for the whole process, the median
    over three environment shifts (a padding variable of 0, 100 and 1,000
    bytes). A start's count follows the environment's size, as the s3 row
    followed its script path's length:

    | start | spread over the three shifts |
    |---|---:|
    | interim | 0.01% |
    | rdb, JIT on | 0.13% (92.47M–92.59M) |
    | rdb, JIT off | 0.13% |

  - **peak anonymous memory**, exactly:
    - Linux keeps a high-water mark of the whole RSS (VmHWM) but none of
      its anonymous part, and a 20 ms start is too short to sample /proc.
    - Anonymous memory grows only by page faults and shrinks only by system
      calls (munmap, madvise, brk, mremap, exit). So the child is stopped at
      every system call under ptrace (`PTRACE_SYSCALL`) and its anonymous
      memory read there; the largest reading is the peak.
    - The reading is `/proc/PID/smaps_rollup`'s `Anonymous`, which the
      kernel counts by walking the page tables (Sol, round 2).
      `/proc/PID/status`'s RssAnon is a batched per-CPU counter: on a
      24-CPU 6.x kernel it can lag by up to about 192 KiB, more than the
      gate's 64 KiB floor. On main's starts the two gave the same numbers.
    - It holds for one thread with no swap:
      - memory the kernel takes back by itself between two stops (swapping
        out, splitting an underused huge page) makes it read low; perf-rt's
        runners have no swap;
      - a start that starts a thread is refused when the thread is made
        (`PTRACE_O_TRACECLONE`), since its memory would be missed even if it
        were joined before exit. robigo-rt's and GNU R's starts start none.
    - A start makes a few thousand system calls; the stops cost well under
      a second per run.
    - Two readings were tried and rejected first:
      - RssAnon at the exit stop is no measure, because robigo-rt has freed
        most of its heap by then (1.3–1.7 MB left).
      - VmHWM less the file-backed pages at exit reads high when file pages
        are reclaimed or unmapped after the peak (Sol, round 1).

      The exact peak is above both (main's interim 1,884 KiB against 1,548
      by the second).
  - **peak RSS:** VmHWM at the exit stop (`PTRACE_EVENT_EXIT`, before the
    kernel tears the memory down), from separate runs stopped only there.
    The system-call stops change which file pages are faulted in: VmHWM
    spread up to 7% under them, and repeats without them.
- **Address randomization is off** for the measured child
  (`personality(ADDR_NO_RANDOMIZE)`, as `setarch -R`). With it on, the
  readings spread widely (the older VmHWM-less-file reading over 15 runs of
  one binary: interim 13.6%, rdb JIT on 5.3%, rdb JIT off 4.7%). With it
  off and the method above, main 90cab787a's 15 runs of each kind gave:

  | start | anonymous peak | peak RSS |
  |---|---:|---:|
  | interim | 1,884 KiB every run | 9,228–9,336 KiB |
  | rdb, JIT on | 6,928 KiB every run | 15,372 KiB every run |
  | rdb, JIT off | 4,632 KiB every run | 13,068 KiB every run |

  Each number is the median of `--startup-runs` runs (default 9) of its
  kind. The child fails loudly if the kernel refuses the personality;
  perf-rt's container runs with `seccomp=unconfined`.
- **The harness's own safety** (Sol, rounds 1–3):
  - the child's output goes to a temporary file, never a pipe, so a start
    that prints much cannot block before its exit;
  - the parent's side is in `try/finally`. On any exception it resumes a
    child still traced (it may sit in a stop already consumed, where a
    SIGKILL is dropped), kills it, and resumes it through its exit stop
    until it and any traced thread are reaped;
  - an exec has its own event stop (`PTRACE_O_TRACEEXEC`), so a real
    SIGTRAP is passed on to the child, as every other signal is;
  - a run over 300 s is killed through a pidfd, so never a process that
    reused the pid, and the timeout is recorded under a lock rather than
    inferred;
  - a main or GNU R record without one of the three numbers is not judged.
- **The gate against main's binary** (the foreman's decision):
  - instructions +0.5%, as the rows;
  - peak anonymous memory +1%, at least 64 KiB;
  - peak RSS +5%, loose, because its file-backed part follows code layout
    (#959: 45.8 KB of code, +0.2–0.4 MB of RSS, anonymous memory
    unchanged).

  H measured single runs moving ±20 KiB on a 2 MB start with randomization
  on, so a bare 1% would sit at the noise floor; the floor covers that.
  With randomization off the readings repeat, so the limits are well above
  the noise.
- **GNU R's starts** (JIT on and off) are measured by `perf.py rt-gnur` on
  the runner, cached with GNU R's rows, and shown beside robigo-rt's, report
  only: the distance to L5's condition, "at or below GNU R's startup".

### Numbers

The dense-tables binary (#974) against main 90cab787a by `perf.py rt-bars`,
0 failures; measured with the earlier anonymous reading, the instructions
and the change being what the method change leaves as they were:

| start | instructions | main | change |
|---|---:|---:|---:|
| interim | 15,431,037 | 15,437,814 | −0.04% |
| rdb-jit | 92,591,129 | 95,330,880 | −2.87% |
| rdb-nojit | 49,985,536 | 50,830,497 | −1.66% |

Main 90cab787a by the final method: the table above (anonymous peak and
peak RSS), with instructions 15,434,183, 95,327,128 and 50,828,824.

GNU R 4.6.2, the local build (`rt-startup.py --gnur`):

| start | instructions | anon KiB | peak RSS KiB |
|---|---:|---:|---:|
| JIT on | 87,633,359 | 10,056 | 22,452 |
| JIT off | 55,066,605 | 6,804 | 19,200 |

## The heap-shift median for allocating rows

- **What happened:** #974's s3 row was +1.19% (16,364 → 16,559) only when
  its scripts sat at `idiom-vs-r.py`'s 19-character temporary path. All 203
  extra instructions per iteration were in glibc's malloc and free: the
  path's length moved the initial heap, and that binary's layout took a
  slower malloc path.
- **The rule now:** a row whose loop calls the C library's allocator
  (`malloc`, `calloc`, `realloc`, `free` calls in the 2N run minus the N
  run, read from its k = 0 callgrind files) at all is measured over the heap
  shifts and reports the median, as an `address_sensitive` row always was
  (`idiom-vs-r.py`'s `follows_heap`).
  - There is no floor on how often (Sol): one rare `realloc` that extends in
    place in one heap can copy a large buffer in another. One call per
    2,000 iterations copying 200,000 instructions is 100 per iteration, 0.6%
    of a 16,000-instruction row. So one layout's count cannot exempt a row.
  - The JSON records each row's `allocator_calls`.
- **Checked:**
  - s3 at the harness's own path on the dense-tables binary: shift 0 is
    16,364.0, the other four 16,355.6–16,355.8. The median, 16,355.7, equals
    main's 16,355.8.
  - plain's loop calls no allocator, so it stays one measurement.
- **Cost:** 19 rows are now measured over shifts in perf-rt, where two were
  marked before. perf-rt took 2 min 49 s on #974; the PR reports the new
  time.
- **The cure for s3 itself** is in code: its loop's three allocations
  (S1's #975). The rule covers the rows whose loops still allocate. Before
  and after, at the harness's own path, each binary from its own trees:

  | s3 | allocator calls per iteration | instructions per iteration |
  |---|---:|---:|
  | #974's binary, rdb | 6.0 | 16,559 alone; median over the shifts 16,355.7 |
  | main b84740012 (#975 in), interim | 0.0005 | 16,103.6, the shifts within 0.6 |
  | main b84740012 (#975 in), rdb | 0.0005 | 16,174.0, the shifts within 8 |

  After #975, s3's loop calls the allocator once per 2,000 iterations and
  stays measured over the shifts, at the cost of four more measurements of
  it. Its shifts agree to within 0.6 and 8 instructions, which is the
  evidence that it no longer follows the heap, not an exemption.

## Tests

`scripts/test-rt-startup.py`, run by perf-rt before the bars:
- `memory()` on small programs:
  - a 2 MB output comes back whole and does not block the child;
  - 64 MB touched and freed before exit still reads as the peak;
  - an exit status comes back, with and without the system-call stops, and
    no child is left;
  - a real SIGTRAP ends the child, as its default action does;
  - a child that starts a thread is refused, whether the thread is alive
    at exit or was joined long before it;
  - a run over its time is killed;
  - a reading that fails at a system-call stop, or at the exit stop, leaves
    no child behind;
- the gate's three limits at and over their edges, including the
  anonymous floor;
- that a start main lacks, wholly or in part, is not judged;
- that the reserved starts refuse to run;
- the table;
- the allocator count from callgrind's compressed names.

`scripts/test-perf-rt-bars.py` passes unchanged.
