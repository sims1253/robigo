# corpus-scripts: whole R programs as a measuring stick

Track: `corpus-scripts` (`docs/workstreams.md`). Goal: measure the project
goal — *Max's existing R scripts run unchanged on robigo* — directly, with
corpora of realistic, whole R programs recorded against GNU R 4.5.0, and
give every other track a ranked list of what breaks real code.

Metric: the corpora exist, are deterministic and run on CI, the triage
below exists; then robigo's pass count on them.

## 2026-10-28: the `gnur-demos` and `r-intro` corpora

- why: the existing corpora are 31k one-line FastR snippets, GNU R's
  regression tests and help-page examples. None of them is an everyday
  analysis script: they have no top-level flow, few compound expressions,
  and they do not combine model fitting, printing of complex objects and
  graphics in one session. So the goal could not be measured.
- what:
  - `tests/corpora/gnur-demos/` (16 cases): every `demo()` shipped with GNU
    R 4.5.0's base packages (`src/library/*/demo/*.R`), run as
    `demo(topic, ask = FALSE, echo = TRUE)` runs it: `options(keep.source,
    useFancyQuotes, warn = 1)` + `set.seed(123)` + the `demo()` banner, then
    the demo source, in echo mode (the transcript `R CMD check` compares).
    The four `tcltk` demos are excluded (they need a display).
  - `tests/corpora/r-intro/` (13 cases): the runnable examples of "An
    Introduction to R" (`doc/manual/R-intro.texi`), one script per chapter
    plus the appendix A sample session. Transcript commands are recovered
    from `> ` lines (continuations are recognised by open brackets, since
    the manual aligns them instead of using `+ `); bare blocks must parse as
    R; a block is dropped when it needs a terminal, browser, network or a
    data file, or prints the time; then each chapter is run under GNU R and
    the block it dies in is dropped until the chapter runs through, so every
    case is a session that completes in GNU R.
  - `scripts/import-corpus-scripts.py` imports both from the R sources;
    `scripts/triage.py` reports the first difference per failing case
    (`--summary` for the how-far-does-it-get table).
  - `corpora.toml`, `tests/corpora/README.md`, `docs/verification.md` and
    one matrix entry per corpus in `.github/workflows/ci.yml` (minimal,
    called out in the PR) wire them in, with the initial `passing.txt`
    ratchet.
- determinism: `record --repeat 2` (the default) gives byte-identical
  output for all 29 cases — no masked lines, no `unstable` cases. Only
  `rnorm`/`runif` vary and the `set.seed(123)` preamble pins them.
- result (`scripts/corpus.py record` in `robigo-oracle:4.5.0`, then
  `run --jobs 4` with `R_HOME` set to the host's 4.5.0):
  **gnur-demos 2/16, r-intro 3/13 = 5/29** at the start. Passing today:
  `base/scoping`, `stats/glm.vr`, `01-introduction-and-preliminaries`,
  `03-objects-their-modes-and-attributes`, `04-ordered-and-unordered-factors`.

## Ranked triage (2026-10-28)

First differing printed line per case, from `scripts/triage.py`, and the
root cause. `first diff` is *line of the recorded output / output lines*:
how far the case gets before it breaks.

### 1. Graphics and grDevices native routines are missing (16 cases)

`par()` (`.External2(C_par)`), `plot.new` (`.External2(C_plot_new)`),
`dev.hold` (`.External(C_devholdflush)`), `colors()` (`.Call(C_colors)`),
`hsv()` (`.Call(C_hsv)`), `stem()` (`.Call(C_StemLeaf)`): every graphics
call in robigo dies in the first native routine it reaches, so every demo
and manual chapter that plots stops there.

| case | first diff | dies in |
|---|---|---|
| `base/recursion` | 67/96 | `plot` → `dev.hold` (C_devholdflush) |
| `grDevices/colors` | 24/133 | `colors` (C_colors) |
| `grDevices/hclColors` | 61/76 | `plot` → `dev.hold` |
| `graphics/Hershey` | 57/674 | `par` (C_par) |
| `graphics/Japanese` | 59/729 | `par` (C_par) |
| `graphics/graphics` | 26/158 | `par` (C_par) |
| `graphics/image` | 22/90 | `terrain.colors` → `hsv` (C_hsv) |
| `graphics/persp` | 33/99 | `par` (C_par) |
| `graphics/plotmath` | 58/240 | `par` (C_par) |
| `stats/lm.glm` | 63/734 | `plot` → `dev.hold` |
| `stats/nlm` | 34/158 | `contour` → `plot.new` (C_plot_new) |
| `stats/smooth` | 16/150 | `par` (C_par) |
| `r-intro/05-arrays-and-matrices` | 40/46 | `plot` → `dev.hold` |
| `r-intro/10-statistical-models-in-r` | 82/136 | `plot` → `dev.hold` |
| `r-intro/11-graphical-procedures` | 9/15 | `par` (C_par) |
| `r-intro/appendix-a-sample-session` | 9/251 | `plot` → `dev.hold` |

- owner: **core/G2** — the native routines of `graphics`/`grDevices` come
  from GNU R's own `graphics.so`/`grDevices.so` once robigo exports
  `libR.so`; they are not to be ported (`docs/workstreams.md`). No action
  for any other track.
- note: this is 16 of the 24 failing cases; the script corpora will jump
  the day G2 lands. Until then they measure graphics only through it.

### 2. `sort()` ignores the locale's collation (2 cases)

`ls(2)` prints `abline, arrows, assocplot, axis, Axis, axis.Date, ...` in
GNU R; robigo prints `Axis, abline, arrows, assocplot, axis, ...`.
GNU R's `sort()`/`order()` on character vectors compare with `strcoll` in
the locale (`src/main/sort.c:scollate`) unless `method = "radix"`: in
glibc's `C.UTF-8`, `sort(c("Axis","abline","axis.Date","axis"))` is
`abline axis Axis axis.Date`, while `sort(..., method="radix")` is
`Axis abline axis axis.Date`. robigo compares bytes, i.e. always radix.
`r-intro/09` shows the same cause in `methods(class="data.frame")`.

| case | first diff |
|---|---|
| `r-intro/06-lists-and-data-frames` | 28/52 (`ls(2)`) |
| `r-intro/09-writing-your-own-functions` | 103/170 (`methods(class=)`) |

- owner: **conformance-vectors** (`builtins/sort.rs`, `TestBuiltin_(sort|order)`).
- next step: port `scollate`/`strcoll` collation for the `auto`/`shell`
  methods and keep byte order for radix; the 09 case also needs `methods`
  (S4, G6) for the *content* of the list, but its first difference is the
  order.

### 3. `parse(keep.source = TRUE)` attaches no source references (1 case)

`base/error.catching` fails at 154/183 lines: GNU R's `str()` of a caught
condition shows the condition's `call` with
`attr(*, "srcref")= 'srcref' int [1:8] ...` and
`attr(*, "srcfile")=Classes 'srcfilecopy', 'srcfile' <environment: ...>`.
robigo's `parse(text=..., keep.source=TRUE)` attaches no `srcref` at all
(`attr(parse(text="f(1)", keep.source=TRUE)[[1]], "srcref")` is `NULL`).

- owner: **parser** (`builtins/parsing.rs`, `srcref`/`srcfile` support;
  files nobody owns today). It is also what `source(echo = TRUE)` needs to
  echo original text instead of deparsing, and what `match.call`/`sys.call`
  printing shows, so it pays off beyond this case.
- next step: give `parse(keep.source=TRUE)` R's `srcref` attributes and
  `srcfilecopy` environments (`src/main/srcref.c`, `srcfile.c`), then
  `str()`/`print()` of language objects will show them.

### 4. The base object census: `.Device`, `.Devices`, `.Last.value`, `@<-` (1 case)

`base/is.things` counts the objects in `package:base`: GNU R 1405 / 1360
functions, robigo 1402 / 1359. The four names GNU R has and robigo does
not: `.Device`, `.Devices` (device state — graphics, G2), `.Last.value`
(the value of the last top-level expression, set by the evaluator),
`@<-` (S4 replacement, G6/methods). The case also misses GNU R's warning
`is.na() applied to non-(list or vector) of type 'expression'` (first
stderr difference): robigo returns `FALSE` silently.

- owners: **perf-vm/cli** for `.Last.value` (top-level evaluation),
  **core/G2** for `.Device`/`.Devices`, **methods/G6** for `@<-`,
  **conformance-values** for the `is.na()` warning.
- next step (smallest first): emit the `is.na()` warning
  (`src/main/logic.c:do_isna` / `coerceVector` check) and set `.Last.value`
  after each top-level expression (`src/main/main.c:R_ReplIteration`-ish);
  both are a handful of lines and make this case's first difference move
  from line 27 to the census counts.

### 5. Complex `sqrt()` is unimplemented (1 case)

`r-intro/02` stops at `sqrt(-17+0i)`: `Error in sqrt(-17 + (0+0i)) :
unimplemented complex function`. GNU R prints `[1] 0+4.123106i`.

- owner: **conformance-values** (`builtins/math.rs`, `TestBuiltin_log`).
- next step: port the complex branch of `do_math1`/`csqrt`
  (`src/main/complex.c`); check `log`, `exp`, `sin`... for the same gap.

### 6. `.Internal(file.info)` is missing, so `data()` fails (1 case)

`r-intro/07` stops at 9/153 lines: `data()` calls `file_test()` →
`file.info()`, and `.Internal(file.info)` does not exist. GNU R's `data()`
then prints the whole `datasets` index — a good startup/lazy-loading case.

- owner: **startup** (`builtins/io.rs`/`sysutils.rs`, unowned files).
- next step: port `do_fileinfo` (`src/main/platform.c`): a data frame with
  `size, isdir, mode, mtime, ctime, atime, uid, gid, uname, grname`, `NA`
  rows for missing files, POSIXct times.

### 7. `loadedNamespaces()` order and content (1 case)

`r-intro/12` first differs at 12/16 lines: robigo prints
`base, datasets, utils, grDevices, graphics, stats` (reverse attach order,
7 entries in GNU R including `compiler`); GNU R prints
`compiler, graphics, utils, grDevices, stats, datasets, base`.

- owner: **startup** (namespace registry / `loadedNamespaces()`).
- next step: return namespaces in load order and load `compiler` (GNU R's
  `R_DEFAULT_PACKAGES` does not include it, but base's startup attaches it
  as a namespace).

### Not yet covered

- `cran-purer`: help-page examples and `tests/` of pure-R CRAN packages
  (R6 classes, environments, closures). Needs network in the recording
  container and package installation time; see the next entry.
- `parse-dumps` for the two corpora (`scripts/parse-corpus.py`): the parser
  track's metric includes every case of `tests/corpora`; adding the corpora
  to `corpus_paths()` and recording dumps in the oracle is a small follow-up
  (needs one `record` run in the container).
