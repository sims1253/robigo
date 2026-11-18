## 2026-11-18: options(keep.source =) honoured as FALSE (L)

- What: `options(keep.source = FALSE)` is accepted (it was refused as an
  option robigo-rt did not honour; base's `sys.source`, which
  `loadNamespace` calls, sets it). GNU R's C reads `keep.source` in two
  places only: the console's parse (gram.c's `R_Parse1Buffer` keeps source
  references when it is TRUE) and `edit()`. `R_KeepSource`, which options.c
  sets, is read nowhere. `parse`, `source` and `sys.source` read the option
  in R, so FALSE needs no C state.
- `keep.source.pkgs` starts as `R_KEEP_PKG_SOURCE == "yes"` (InitOptions);
  it was always FALSE.
- Known gap: `options(keep.source = TRUE)` is refused ("robigo:
  options(keep.source = TRUE) (source references) is not implemented yet"),
  as are `parse(keep.source = TRUE)` and the function srcrefs it would make.
  `source()` needs utils' namespace (`utils::localeToCharset`), which
  robigo-rt cannot load yet.
- Case: `tests/rt/options/keep-source.R`; the refusal and the environment
  variable in `crates/robigo-rtapi/tests/cli.rs`.
