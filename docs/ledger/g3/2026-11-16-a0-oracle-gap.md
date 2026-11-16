## 2026-11-16: A0, the oracle gap, and what a stock GNU R does to the M1 bars

The first item to settle before M2/M3 planning (`2026-11-16-m1-signoff.md`),
as the M2/M3 plan's §E.a words it. The oracle image's GNU R counts 3.9% to
12.9% more instructions than the developer machine's local build
(`2026-11-16-bars-oracle-baseline.md`). The plan's prediction was that
Debian's build flags explain most of it. Under the lead's F2 (A), the
rival for the instruction bars becomes "a stock build from the pinned
tarball, with GNU R's default configure flags, in the CI image's
toolchain", so this entry also measures that build against robigo-rt.

**No bar's status changes here.** The bars file, the gate and CI are as
they were. The foreman decides what follows (the last section).

**Results:**
- **The gap is the toolchain and the flags.** Of the oracle image's 7.0%
  (the geometric mean over 12 rows):
  - the toolchain (gcc 14.2 with glibc 2.41, against gcc 16.2.1 with
    glibc 2.44) explains 3.7% to 4.1%, and it is mostly the compiler;
  - Debian's hardening flags explain 2.4% to 2.7%;
  - the configure options, memory profiling among them, explain nothing,
    except on the two rows that hash strings.

  The prediction holds for s3 (flags 2.5%, toolchain 1.2%) but not
  overall. The plan names "A0 blaming the toolchain rather than the
  flags" as a re-plan trigger.
- **GNU R's default build is a static libR.** It counts 7.0% fewer
  instructions than the same build as a shared library, and 9.6% fewer
  than the oracle image. That is the largest factor here, and the plan's
  estimate did not include it.
- **Against that build, s3 reads 1.096x**, not the plan's "about 1.01x".
  1.017x is what removing the flags alone gives. The other five E1 bars
  stay met.
- **e7-unique's and e7-match's GNU R counts move with the heap layout**,
  by up to 5%, because GNU R hashes strings by address.

### The cells

Seven builds of GNU R 4.6.2, all measured on m0pad (AMD Ryzen 5 PRO
5650U). Six are A0's builds of the pinned tarball. The seventh is the
oracle image, which builds the same tarball.

| cell | toolchain | flags | configure |
|---|---|---|---|
| oracle | the oracle image (Debian trixie: gcc 14.2, glibc 2.41) | Debian's | Debian's r-base set (`.github/ci-image/oracle.Dockerfile`) |
| img-deb | Debian trixie, as the image | Debian's | A0's fixed set |
| img-plain | Debian trixie | `-g -O2` | A0's fixed set |
| img-stock | Debian trixie | `-g -O2` | GNU R's defaults: F2 (A)'s rival |
| host-deb | m0pad's CachyOS: gcc 16.2.1, glibc 2.44 | Debian's | A0's fixed set |
| host-plain | CachyOS | `-g -O2` | A0's fixed set |
| host-local | CachyOS | `-g -O2` | the local build's: the fixed set without memory profiling |

- **Debian's flags:** `-g -O2 -ffile-prefix-map=/=. -fstack-protector-strong
  -fstack-clash-protection -Wformat -Werror=format-security -fcf-protection
  -Wdate-time -D_FORTIFY_SOURCE=2`, linked with `-Wl,-z,relro`, as
  `dpkg-buildflags` gives them on trixie.
- **A0's fixed set:** `--enable-R-shlib --enable-memory-profiling
  --without-recommended-packages --disable-java --with-x=no`, with
  readline, Tcl/Tk, cairo, libtiff, libjpeg and libpng off. Every cell
  except oracle and img-stock uses it.
- **img-stock:** `--without-recommended-packages --disable-java
  --with-x=no` and nothing else. GNU R's defaults leave
  `--enable-R-shlib` and memory profiling off, so libR is linked into the
  `R` executable. The three options only remove packages, Java and the X11
  device, which these rows never load.
- **Valgrind:** Debian's 3.24.0 in the image cells and CachyOS's 3.25.1 on
  the host. The "toolchain" factor therefore includes valgrind's version.
  That version can move a count only through the CPU features it reports
  to glibc. The C library's row below nets out across s3 and fib.

Each build records its `CC`, `CFLAGS` and `LDFLAGS` from `Makeconf`
(`a0-flags.txt` in the image, the configure logs on the host). A0's
scripts are in the data directory, `data/2026-11-16-a0-oracle-gap/scripts/`,
and are named below by their file names.

### Method

- **The rows:** `scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml
  --no-robigo`, the bars tool's N against 2N under callgrind, on the 12 rows
  of `2026-11-16-bars-oracle-baseline.md`. They ran twice per cell. The two
  runs agree within 0.0004% on every row and cell.
- **Reproduction:**
  - **host-local is the local build.** It matches the developer machine's
    local build (that entry's table) within 0.04% on 10 of the 12 rows.
    The exceptions are e7-match, at −0.13%, and named, at −1.3%. Nothing
    here explains named.
  - **The oracle cell is the committed baseline.** It matches the bars
    file's `[gnur]` numbers within 0.12% on 9 of its 11 rows. The
    exceptions are the two hashing rows, e7-match at −0.9% and e7-unique
    at −0.27% (see "The hashing rows").
- **The causes:** `scripts/callgrind-diff.py --gnur --causes`, new here,
  on s3 (4,000 iterations against 8,000) and fib (20 calls of fib(18)
  against 40), from the data directory's `rows/`. Its new
  `benchmarks/causes-gnur.toml`
  splits a row's instructions by GNU R's source file.
- **robigo-rt:** CI's binary from #746's run (the `perf-rt-bins` artifact,
  main as of `3a9442c3`). It ran on the same machine, inside the oracle
  image with valgrind (the `perf-rt` job's environment), twice. Its
  counts agree with CI's on the EPYC 9V74 within 0.05%.

### Instructions per iteration

| row | oracle | img-deb | img-plain | img-stock | host-deb | host-plain | host-local |
|---|---:|---:|---:|---:|---:|---:|---:|
| plain | 4,800 | 4,799 | 4,693 | 4,275 | 4,591 | 4,474 | 4,472 |
| defaults | 6,088 | 6,088 | 5,946 | 5,449 | 5,791 | 5,645 | 5,644 |
| named | 4,639 | 4,639 | 4,547 | 4,187 | 4,496 | 4,376 | 4,375 |
| env-fn | 3,832 | 3,832 | 3,739 | 3,389 | 3,711 | 3,619 | 3,619 |
| superassign | 7,838 | 7,838 | 7,598 | 6,892 | 7,698 | 7,485 | 7,480 |
| s3 | 19,191 | 19,209 | 18,736 | 17,392 | 18,989 | 18,518 | 18,503 |
| trycatch | 93,912 | 93,895 | 91,857 | 84,944 | 90,900 | 88,516 | 88,501 |
| fib | 24,056,138 | 24,056,196 | 23,537,699 | 21,255,251 | 23,300,900 | 22,723,897 | 22,723,897 |
| binarytrees | 3,781,021 | 3,781,023 | 3,702,889 | 3,373,969 | 3,666,113 | 3,571,974 | 3,571,758 |
| e7-paste | 195,850 | 195,875 | 187,084 | 177,054 | 186,746 | 177,497 | 177,407 |
| e7-match | 205,949 | 207,231 | 204,699 | 203,418 | 195,529 | 191,465 | 191,433 |
| e7-unique | 170,397 | 161,946 | 159,447 | 158,017 | 153,907 | 151,330 | 151,310 |

### The factors

Each factor is the ratio of two cells that differ in one thing.
`factors.md` in the data directory has every row.

| factor | cells | s3 | fib | the 12 rows | geomean |
|---|---|---:|---:|---|---:|
| Options: Debian's r-base set against A0's | oracle ÷ img-deb | −0.1% | −0.0% | −0.6% to +5.2% (the hashing rows); the rest ±0.1% | +0.4% |
| Debian's flags, image toolchain | img-deb ÷ img-plain | +2.5% | +2.2% | +1.2% to +4.7% | +2.4% |
| Debian's flags, host toolchain | host-deb ÷ host-plain | +2.5% | +2.5% | +1.7% to +5.2% | +2.7% |
| Toolchain, with Debian's flags | img-deb ÷ host-deb | +1.2% | +3.2% | +1.2% to +6.0% | +3.7% |
| Toolchain, plain flags | img-plain ÷ host-plain | +1.2% | +3.6% | +1.2% to +6.9% | +4.1% |
| Memory profiling | host-plain ÷ host-local | +0.1% | +0.0% | +0.0% to +0.1% | +0.0% |
| **The whole gap** | oracle ÷ host-local | +3.7% | +5.9% | +3.7% to +12.6% | +7.0% |
| Shared libR against GNU R's static default | img-plain ÷ img-stock | +7.7% | +10.7% | +0.6% to +10.7% | +7.6% |

The factors multiply: options × flags × toolchain × memory profiling gives
the whole gap to within 0.02 points (1.004 × 1.024 × 1.041 × 1.000 =
1.070).

On the host, most of the flags' cost on s3 (392 of 477 instructions an
iteration) is in code that callgrind maps to no object (`???`, in
valgrind's own address range). It appears in host-deb only. The image's
flags factor has no such row and agrees on s3 (2.5%).

### Where the instructions go

`callgrind-diff.py --gnur --causes`, by GNU R's source file. "flags" is
img-deb against img-plain, "toolchain" img-plain against host-plain, and
"shared libR" img-plain against img-stock. The C library and the dynamic
linker share a row, because the host's valgrind charges `__sigsetjmp` to
ld.so and the image's charges it to libc.

**s3**, instructions per iteration:

| cause | img-deb | img-plain | img-stock | host-plain | flags | toolchain | shared libR |
|---|---:|---:|---:|---:|---:|---:|---:|
| memory.c | 6,995 | 6,904 | 6,784 | 6,820 | +1.3% | +1.2% | +1.8% |
| eval.c | 3,290 | 3,187 | 2,855 | 3,197 | +3.2% | -0.3% | +11.6% |
| envir.c | 3,139 | 3,058 | 2,745 | 3,104 | +2.6% | -1.5% | +11.4% |
| src/include, inlined | 1,410 | 1,401 | 1,240 | 1,387 | +0.6% | +1.0% | +13.0% |
| the rest of src/main | 1,111 | 1,041 | 978 | 1,063 | +6.7% | -2.1% | +6.4% |
| match.c | 834 | 799 | 724 | 800 | +4.4% | -0.1% | +10.4% |
| objects.c | 829 | 804 | 724 | 813 | +3.1% | -1.1% | +11.0% |
| errors.c, context.c | 585 | 566 | 449 | 548 | +3.4% | +3.3% | +26.1% |
| libc and ld.so | 523 | 486 | 487 | 433 | +7.6% | +12.2% | -0.2% |
| attrib.c | 498 | 483 | 399 | 334 | +3.1% | +44.6% | +21.1% |
| outside GNU R's sources | 29 | 0 | 0 | 0 | +29 | +0 | +0 |
| smaller causes | 0 | 7 | 7 | 6 | -7 | +1 | +0 |
| **Total** | 19,243 | 18,736 | 17,392 | 18,505 | +2.7% | +1.2% | +7.7% |

**fib**, instructions per call of fib(18):

| cause | img-deb | img-plain | img-stock | host-plain | flags | toolchain | shared libR |
|---|---:|---:|---:|---:|---:|---:|---:|
| eval.c | 12,466,051 | 12,332,274 | 11,048,878 | 11,813,963 | +1.1% | +4.4% | +11.6% |
| memory.c | 3,639,591 | 3,555,970 | 3,426,574 | 3,352,404 | +2.4% | +6.1% | +3.8% |
| envir.c | 2,474,810 | 2,382,840 | 2,098,571 | 2,315,956 | +3.9% | +2.9% | +13.5% |
| match.c | 2,173,858 | 2,073,526 | 1,872,862 | 2,081,887 | +4.8% | -0.4% | +10.7% |
| src/include, inlined | 1,654,090 | 1,629,007 | 1,486,874 | 1,578,847 | +1.5% | +3.2% | +9.6% |
| errors.c, context.c | 1,254,540 | 1,229,384 | 986,842 | 1,195,977 | +2.0% | +2.8% | +24.6% |
| libc and ld.so | 334,440 | 334,439 | 334,439 | 384,606 | +0.0% | -13.0% | +0.0% |
| outside GNU R's sources | 58,527 | 0 | 0 | 0 | +58,527 | +0 | +0 |
| smaller causes | 288 | 257 | 208 | 257 | +31 | +0 | +49 |
| **Total** | 24,056,196 | 23,537,698 | 21,255,249 | 22,723,897 | +2.2% | +3.6% | +10.7% |

- **The flags cost a little everywhere.** Every one of libR's files grows,
  by 0.6% to 6.7% on s3 and by 1.1% to 4.8% on fib. That fits flags that
  touch every function:
  - `-fcf-protection` puts an `endbr64` at each function entry and
    indirect-branch target;
  - `-fstack-protector-strong` checks a canary in each function with a
    local array or an address-taken local;
  - `-fstack-clash-protection` probes large frames.

  "Outside GNU R's sources" appears in the Debian-flag cells only, and its
  largest function is `Rf_matchArgs_NR`. In img-deb's profile, such lines
  come from glibc's `bits/string_fortified.h`: the wrappers that
  `_FORTIFY_SOURCE=2` inlines.
- **The toolchain is the compiler.** The C library's row goes opposite
  ways on the two rows: +53 on s3, −50,167 on fib. The rest is libR's own
  code: on fib, eval.c +4.4% and memory.c +6.1%; on s3, attrib.c +45%.
- **The shared library costs most in the evaluator.** eval.c, envir.c,
  match.c and objects.c each grow by 10% to 13.5%, and the contexts by
  25%. memory.c grows by only 2% to 4%.

### The static build

GNU R's default links libR into the `R` executable. Debian, and every
other cell here, builds `libR.so` with `-fpic`. In an ELF shared library,
every call from libR to one of its own exported functions goes through the
PLT, and gcc cannot inline such a function, because another library could
interpose it (`-fsemantic-interposition`, gcc's default). The disassembly
shows it (`pic-check.sh`, output in `pic-check.out`):

| | img-plain: `libR.so` | img-stock: `R` |
|---|---:|---:|
| PLT call sites | 33,085 | 7,412 |
| `bcEval_loop`: instructions, PLT calls | 16,765, 510 | 15,631, 42 |
| `Rf_eval` | 497, 22 | 436, 2 |
| `Rf_findVarInFrame3` | 254, 10 | 196, 2 |
| `Rf_begincontext` | 91, 1 | 72, 0 |

The cost is spread over libR's code (the table above). The profiles show
no PLT stubs of their own (callgrind's `--skip-plt=yes`, the default), so
the tables cannot split the stubs from the lost inlining. robigo-rt is one executable and pays neither.

### The hashing rows

The options factor is zero on every row but e7-unique (+5.2%) and e7-match
(−0.6%). GNU R hashes a cached string by its CHARSXP's address
(`unique.c:205`, `cshash`; `shash` uses it for every cached string,
`unique.c:228`). So the number of collisions, and the count, follows
where the strings were allocated. To test it, `layout-rows.toml`
allocates 0, 1, 7, 50 or 333 unrelated strings before each row's setup:

| unrelated strings first | 0 | 1 | 7 | 50 | 333 |
|---|---:|---:|---:|---:|---:|
| e7-unique, oracle | 170,397 | 171,308 | 170,845 | 161,939 | 161,943 |
| e7-unique, img-deb | 161,946 | 161,934 | 161,927 | 161,949 | 161,938 |
| e7-match, oracle | 205,949 | 206,546 | 205,479 | 208,686 | 206,658 |
| e7-match, img-deb | 207,231 | 206,810 | 207,153 | 207,999 | 206,384 |
| e7-unique, robigo-rt | 126,476 | 126,494 | 126,385 | 126,384 | 126,354 |
| e7-match, robigo-rt | 99,032 | 98,903 | 98,909 | 98,906 | 98,928 |

- **GNU R:** the oracle build's e7-unique drops 5.0% once 50 strings move
  its keys, and e7-match moves by up to 1.3% in either build.
- **robigo-rt:** it ports `cshash`
  (`crates/robigo-rtbuiltins/src/unique.rs:316`), so its counts on these
  rows follow its layout too. Across the same paddings they move by at
  most 0.13% (e7-unique 0.11%, e7-match 0.13%). This was measured on the
  desktop with a release build of main `2ae1e075` (`layout-rt.json`).

Both rows' bars compare robigo-rt with today's robigo, so no bar moves.
A bar against GNU R on a hashing row would carry GNU R's noise.

### What F2 (A) does to the M1 bars

E1's six bars compare with GNU R. robigo-rt's counts and the ratio to
three GNU R builds, all on m0pad:

| row | robigo-rt | bar | ÷ oracle (today's rival) | ÷ img-plain (no flags) | ÷ img-stock (F2 A) |
|---|---:|---:|---:|---:|---:|
| plain | 1,922.1 | 0.6x | 0.400x | 0.410x | 0.450x |
| defaults | 1,999.0 | 0.6x | 0.328x | 0.336x | 0.367x |
| named | 1,848.1 | 0.6x | 0.398x | 0.406x | 0.441x |
| env-fn | 2,592.6 | 1.0x | 0.677x | 0.693x | 0.765x |
| superassign | 5,142.5 | 1.0x | 0.656x | 0.677x | 0.746x |
| **s3** | 19,053.5 | 1.0x | 0.993x | 1.017x | **1.096x** |

- **s3 would be unmet by 9.6%.** The plan's "about 1.01x" is the
  img-plain column, the flags alone.
- **The other E1 bars stay met.** The closest is env-fn, at 0.765x of
  1.0x.
- **E2's and E7's bars compare with today's robigo**, so F2 changes none of
  them.
- **The RSS bars:** GNU R's peak RSS varies by at most 1.5% across the
  image cells (`scripts/rss.py`, five runs each). robigo-rt's peak RSS is
  15% to 45% below img-stock's, so every RSS bar stays met:

  | program | oracle | img-deb | img-plain | img-stock | robigo-rt |
  |---|---:|---:|---:|---:|---:|
  | binarytrees.R | 42,140 | 41,744 | 41,508 | 42,016 | 23,716 |
  | listgrow.R | 54,028 | 53,680 | 53,604 | 54,048 | 45,820 |
  | churn.R | 134,768 | 133,864 | 133,924 | 134,360 | 73,956 |

These are m0pad's numbers. CI measures GNU R per CPU signature (A2), and
robigo-rt's counts on m0pad and on CI's EPYC agree within 0.05%, so CI's
ratios should be within a few tenths of a point of these.

### For the foreman

1. **Which stock build.** F2 (A) as worded is the static default: s3 at
   1.096x, so P3 owes 9.6%, not 1%. The other reading, GNU R's default
   flags with `--enable-R-shlib` as Linux distributions ship it, gives
   1.017x. **Recommended: the static default, as worded.** It is what GNU
   R's own tarball builds, it is the harder rival, and robigo-rt pays no
   interposition cost of its own. Charging GNU R for a packaging choice
   that robigo does not make would flatter robigo-rt.
2. **The toolchain trigger fired.** The toolchain explains more of the gap
   than the flags across the rows, though not on s3. Under F2 (A), the
   rival keeps CI's toolchain (gcc 14.2). gcc 16 would take about 4% more
   off GNU R (1.2% on s3). **Recommended: keep CI's toolchain.** It is
   pinned by the oracle image's digest, which is already in `perf-rt`'s
   cache key, and CI has no newer compiler without another image.
3. **The hashing rows, and X3's instruction guard** ("no instruction row
   worse than +1%"). X3 compares robigo-rt variants, so robigo-rt's
   sensitivity is the one that counts: at most 0.13% across five
   paddings, inside the guard and the gate's 0.5%. A header or node-size
   change moves every address, which padding does not test. So X3 should
   run `layout-rows.toml` on each variant before it reads a +1% on
   e7-unique or e7-match as real. A bar against GNU R on a hashing row
   needs a reference that does not depend on one layout. One way is the
   median over a few paddings, as above. The other is a margin wider than
   5%.
4. **CI, once decided:** build the stock GNU R into the oracle image (for
   example `/opt/R-stock`, about three minutes on 12 cores) and point
   `perf.py rt-gnur` at it with `RSCRIPT_BIN` and `R_HOME_DIR`. The
   image's digest keys the cache, so nothing else changes. This is a
   small package of its own. It changes s3's status, so it waits for
   the decision.

### Reproduce

On a machine with docker, a C toolchain, valgrind and the pinned tarball
in `~/robigo-a0`, with a copy of the repository's `scripts/` and
`benchmarks/` in `~/robigo-a0/repo`:
- `build.sh` builds the six cells (about 3 minutes each);
- `measure.sh` runs the rows twice and the cause diffs;
- `measure-rt.sh` runs robigo-rt and the RSS programs, with the GNU R
  sources that `scripts/r-src.sh` extracts;
- `pic-check.sh` counts the PLT calls inside an image cell.

The data directory has every run's JSON and log, the cause tables
(`{cell}-diff-{s3,fib}.txt`), the averaged counts (`instr.json`,
`rt-instr.json`) and the layout runs (`layout-{oracle,deb,rt}.json`).
