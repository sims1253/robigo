# conformance-strings: character, paste, grep, sprintf and friends

Metric: passes in `fastr` within

    TestBuiltin_(paste|substr|sprintf|grep|sub|regexpr|nchar|toupper|
                 strsplit|chartr|format|trimws|startsWith|endsWith|
                 encodeString|agrep|grepRaw|iconv)

Baseline on `main` (10d0a81, 2026-10-28): **1132/1292** (160 failing).
After these three changes: **1256/1292** (36 failing) -- 124 cases fixed,
0 regressions.  Rebased on `bfef3cc` (other tracks landed in between):
**1260/1292** (32 failing); the remaining 32 are iconv (19, blocked
below), formatPOSIXlt (10), format (1), regexpr (1) and subset (1).

## 2026-10-28: agrep/agrepl; grepl `{}` with blanks (commit `4357bee`)

- why: `agrep`/`agrepl` were missing entirely (20 cases), and 4 `grepl`
  cases failed on errors that only PCRE allows.
- what:
  - `agrep.c:do_agrep`, `agrep.c:amatch_regaparams` and the wrappers'
    argument handling in `builtins/grep.rs`.  Note the naming trap: TRE's
    *insert* consumes a text character, so R's `costs[0]`/`bounds[2]` are
    TRE's insertion cost/limit and R's `costs[1]`/`bounds[1]` are its
    deletion ones.
  - matching is a port of TRE's `tre_tnfa_run_approx`, not an
    edit-distance DP (see the rejected experiment below).
  - `grepl`: PCRE2 allows blanks inside a quantifier (`{2,  3}`, `{ 2 }`),
    TRE does not, so the two translators now parse `{}` separately; the
    TRE side is a full port of `tre-parse.c:tre_parse_bound`, including
    the count and cost scans.  `x{2, 3}` is `Invalid contents of {}`
    (TRE reads the `3` as the start of a cost equation) while `x{2, }`
    and `x{2,    }` are valid.
- probes: a 1500-case randomised differential of `.Internal(agrepl(...))`
  against GNU R (patterns and texts over `abc`, both `icase` values,
  random `costs`/`bounds` including `NA`) -- 0 mismatches, after a
  sequence of fixes driven by TRE's own debug build
  (`-DTRE_DEBUG` on `src/extra/tre`) .
- result: `TestBuiltin_(agrep|grepRaw|grepl)` 65/65; fastr 1132 -> 1256.
  kept.

### Experiment rejected: approximate matching as a Pareto edit-distance DP

- why: TRE's matcher is a state machine; a DP looked equivalent and is
  much easier to reason about.
- what: per cell, the Pareto-minimal `(pattern-cuts, text-cuts,
  substitutions)` triples, all operations also counted as errors.
- result: 83/1500 mismatches on the randomised differential.  TRE keeps
  **one** path per state and drops a later path on a cost tie, so the
  answer depends on operation order, and a DP that keeps every
  Pareto-minimal path is strictly more permissive.  rejected.

### Then: a faithful state machine, still 24/1500 wrong

- what: ported the loop order (initial states, inserts, deletes, exact
  matches and substitutions) with TRE's tie-breaks.
- result: 24/1500 mismatches, all with `costs[2] == 0` (free
  substitutions) and almost all `icase = TRUE`.
- cause: with `REG_ICASE`, `tre-parse.c` adds the opposite-case
  counterpoint of each literal and `tre_set_union` keeps the two items as
  two transitions, upper case first.  Both lead to the same state and on
  equal cost the first wins, so a *free* substitution is taken and the
  exact match is discarded -- which also spends an error from
  `bounds[4]`.  Confirmed with the TRE debug build (`ab` vs `aA`: `subst
  ... win` then `exact ... lose`).
- result: 0/1500.  kept.

## 2026-10-28: grepRaw (commit `71f719b`)

- why: `grepRaw` was missing (20 cases).
- what: `grep.c:do_grepraw` plus `fgrepraw1`; verified the `offset`
  arithmetic is `R_size_t` (NA/negative wraps to a huge offset and yields
  `integer(0)`, it is not an "invalid 'offset'" error).
- result: 20/20, fastr +20.  kept.

## 2026-10-28: encodeString (commit `d9d8736`)

- why: 80 cases failed; `encodeString` escaped every quote character, and
  computed widths from the escaped string.
- what: `format.c:encodeString` semantics in `builtins/character.rs`
  (rather than `format.rs`, which belongs to the print track): only `'`,
  `"` and `` ` `` are escaped, and only when they equal `quote`; the
  width adds 2 only when `quote` is non-empty.  Non-printable characters
  are escaped as `\a\b\f\n\r\t\v` and `\ooo`/`\uxxxx`/`\Uxxxxxxxx`.
- result: 80/80; the whole string subset 601/601.  kept.

## Blocked: iconv (19 cases)

17 cases start with `Encoding(x) <- "latin1"` and 2 depend on `LC_CTYPE`
(`Sys.setlocale("LC_CTYPE", "C")` must make `²` unconvertible).  Both
need the same model change, in a crate this track does not own:

- `value.rs` has `RStr = Option<Rc<str>>`; an R string also carries an
  encoding mark (`CE_NATIVE`/`CE_UTF8`/`CE_LATIN1`/`CE_BYTES`), and
  `Encoding<-` must be able to set it.  Without it `iconv` cannot know
  what the input bytes mean, and printing cannot show `<e7>` for bytes.
- `iconv` also needs the locale's `LC_CTYPE` (R calls `iconv` with
  `mbcsToLatin1`/`translateChar` and falls back to `NA` on failure).

Not attempted here; no partial implementation that guesses was left
behind (`Encoding<-` raises `robigo: marking strings as "latin1" is not
implemented yet`, `iconv` to latin1 raises
`robigo: latin1-encoded strings are not implemented yet`).

## Failures outside this track (the other 13 of 32)

Ranked by count, with the first differing stream:

1. **formatPOSIXlt (10)**: `Error: 3 arguments passed to
   .Internal(format.POSIXlt) which requires 4` -- an internal arity error
   is reported with the call context; GNU R reports it bare with `Error:`
   (interp error reporting, lang track).
2. **format (1)**: an internal error's call is displayed as
   `format(...)` where GNU R shows `.Internal(format(...))` (deparse,
   print track).
3. **regexpr (1)**: the parser's escape error text
   (`'\d' is an unrecognized escape in character string (<input>:1:36)`),
   parser track.
4. **subset (1)**: `srcref` attributes from `parse(keep.source=TRUE)`,
   parser track.

(`substitute` (2) and two `subset` cases were fixed by the lang and
vectors tracks while this branch was in review.)
