## 2026-11-18: A4, the builtin differential (`ROBIGO_BUILTIN_DIFF`), out of process against GNU R 4.6.2

- **Why:** docs/g3.md §10 and §16, plan §B "A4" and §F.6 (decided A: out of
  process, on generated inputs). The foreman approved the design note
  (`~/.cache/robigo-research/a4-builtin-diff-design.md`) with:
  - answers to its §10: the records cached by hash; an acceptance seed;
    known lists of case ids; `withVisible` ported by E (#850);
  - amendments:
    - A: a memory cap, the `resource` class, no network;
    - B: the raw `.Internal` call primary, the wrapper as a fourth kind;
    - C: the environment named and keyed;
    - D: the score line.
- **What it is:** docs/verification.md, "Builtin differential".
  `scripts/builtin-diff.py` with `scripts/builtin-diff/{gen,driver,render}.R`,
  `tests/builtin-diff/rows.toml` (22 rows; every other registry builtin
  under `[uncovered]` with its reason), `tests/builtin-diff/known/`, the
  plants behind `robigo-rtbuiltins/diff-plant`, rt-ci.py's step, and CI's
  `builtin-diff-data` job.
- **The bar** (`plant`): all five plants fail in their own field, the panic
  costs only its own cases, and the control row (`rep_len`) is unchanged:

  | plant | new failures | field |
  |---|---|---|
  | nchar | 1,111 | value, error |
  | seq_len | 30 | conds |
  | names | 1 | env |
  | abs | 1 | crash |
  | set.seed | 26 | seed |

- **Two things the measurements found in the harness's own path:**
  - **The formula carried the lattice.** The formula value had the
    generator's frame as its environment, which holds the whole lattice. A
    batch was 99 MB and a record up to 50 MB; GNU R's cache reached 60 GB.
    The formula's environment is now base's, the values are kept once per
    batch, and an unchanged binding is recorded as TRUE.
  - **robigo-rt's file connections fsynced on close.** `close_conn` called
    `sync_all()`, about 15 ms per closed file here, which every R script
    writing files pays. GNU R's `file_close` is `fclose`, which does not
    sync; robigo-rt's file writes are unbuffered, so nothing is left to
    flush. Ported as such (robigo-rt/src/connections.rs).

  A full run went from 5 min 17 s to 40 s. Its breakdown, at 4 jobs:
  - robigo-rt's driver, 96 s summed (start 0.06–0.3 s; about 1.3 ms a case);
  - the comparison, 2.5 s;
  - render.R, 1.2 s;
  - GNU R's side on a cold cache, 60 s more. It runs each batch twice.
- **GNU R's own nondeterminism:** 21 RNGkind and 26 set.seed cases seed from
  the clock (`set.seed(NULL)`, no `.Random.seed`). Before the double run
  they looked like seed mismatches. Now they are excluded as unstable,
  counted with oracle-crash.
- **First findings** (in the rows' known lists, not fixed here):
  - **`nchar(y ~ x)`:** GNU R `c(1L, 1L, 1L)`, robigo-rt `c(3L, 1L, 1L)`.
    robigo-rt's `as.character(quote(y ~ x))` is `c("~", "y", "x")`, as in GNU
    R, so the 3 is nchar's own coercion of a call (rtbuiltins'
    `Strings::of`).
  - **`rep_len` through `.Internal`:** with a language argument GNU R
    evaluates it a second time (`DispatchOrEval`, argsevald = 0):
    `.Internal(rep_len(quote(x), 1.5))` is "object 'x' not found", while
    robigo-rt says "attempt to replicate non-vector".
  - **A compact sequence of 10⁹ or more elements inside a list is
    expanded** by robigo-rt's serializer. `seq_len(.Machine$integer.max)`
    alone serializes in 133 bytes, but `saveRDS(list(x))` allocates 8 GB.
    Between 10⁸ and 10⁹ elements it stops staying compact.
  - **`seq_len(<raw>)`:** GNU R gives `error()` "unimplemented type 'raw'
    in 'asReal'\n". robigo-rt gives errorcall "argument must be coercible to
    non-negative integer".
  - **A latin1-marked "\xe9" coerced to a number** (RNGkind, set.seed,
    seq_len, seq.int). GNU R in C.UTF-8 errors "invalid multibyte string at
    '<e9>'"; robigo-rt returns NA with "NAs introduced by coercion".
- **GNU R's crashes:** 40 raw `.Internal(strtoi(x, base))` inputs segfault
  GNU R (oracle-crash). `x` is not a character vector (numeric, logical,
  complex, raw, factor, Date, a matrix) and `base` passes do_strtoi's check
  (`0L`, or an integer vector whose first element is valid, `10:1`). Then
  `STRING_ELT` reads a non-string vector. base's wrapper passes
  `as.character(x)`.
