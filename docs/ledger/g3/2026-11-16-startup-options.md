## 2026-11-16: the options Rprofile.unix sets at startup

- What: the foreman's startup-options item. `options()` in robigo-rt lacked
  the eight entries GNU R 4.6.2's system profile adds after `Common.R`'s:
  `papersize`, `printcmd`, `dvipscmd`, `texi2dvi`, `browser`, `pager`,
  `pdfviewer` and `useFancyQuotes` (`library/profile/Rprofile.unix`). The
  diff was against the oracle's `names(options())` under
  `R_DEFAULT_PACKAGES=NULL`. They are now in `init_options`' list, in
  `Rprofile.unix`'s order after `defaultPackages`.
- Tree: branch `g3/startup-options` on `g3/collation` (#700), because
  `options()`'s order is Scollate's. Binary: `cargo build --release --locked
  -p robigo-rtapi` (`robigo-rt`). Machine: AMD Ryzen 9 5900X (24 threads),
  WSL2 Linux 6.18, rustc 1.100.0. GNU R: the 4.6.2 oracle image.

### The values

`Rprofile.unix` reads the variables that `etc/Renviron` sets with
`${VAR-default}`. Its defaults are the configure values, which in the
oracle are `R_PRINTCMD` `/usr/bin/lpr`, `R_BROWSER` `xdg-open`,
`R_PDFVIEWER` `/usr/bin/xdg-open` and `R_TEXI2DVICMD` `${TEXI2DVI-texi2dvi}`.
Its `pager` is `file.path(R.home(), "bin", "pager")`, with R_HOME
`/usr/lib/R`. robigo-rt has no installation of its own, so it reads the
same variables when they are set and otherwise takes the oracle build's
values, as `.Platform` does (`platform.rs`):

- `printcmd`, `browser`, `pdfviewer`: the variable when it is set, even
  empty, else the configure value;
- `texi2dvi`: `R_TEXI2DVICMD`, else `TEXI2DVI`, else "texi2dvi";
- `dvipscmd`: `Sys.getenv("DVIPS", "dvips")`;
- `papersize`: the user's `R_PAPERSIZE` (Renviron's `R_PAPERSIZE_USER`)
  when not empty, else from `LC_PAPER`. That gives "letter" for a locale
  with `_US` or `_CA` and "a4" otherwise, and the isolate's locales (C,
  C.UTF-8) give "a4";
- `pager`: "/usr/lib/R/bin/pager";
- `useFancyQuotes`: TRUE.

Every option's name, type and value now equals the oracle's under the
conformance environment (a probe over `options()`, not checked in).

### Proof

- `tests/rt/options/startup.R`, recorded on 4.6.2 and ratcheted, checks
  `names(options())`, the values that depend on neither the installation
  nor the environment, and that the installation's commands are strings.
- RTCI; `scripts/lint.sh` clean.

### Open

robigo-rt's `R_HOME` (and `R.home()`, not bound) is a design question that
g3.md does not settle: `pager` names the oracle's.
