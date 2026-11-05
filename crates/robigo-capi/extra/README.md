# GNU R's src/extra: TRE

`tre/` is GNU R 4.5.0's `src/extra/tre`, copied unmodified (BSD-2, see
`tre/LICENSE`; GNU R's changes are in `tre/R_changes`): the sources and
headers its `Makefile.in` builds into `libtre.a`. `build.rs` compiles them
into robigo's libR against the internal headers in `main/include`
(`config.h`, `rlocale.h`), as GNU R does.

robigo's regular expressions (`robigo-runtime`, `builtins/grep.rs`) match
most TRE patterns with their own engine and send the constructs that engine
can get wrong (minimal repetitions, word boundaries, back-references,
ambiguous submatches, approximate matching) to this TRE, called as
`src/main/grep.c` calls it (`src/tre.rs`, the `TreEngine` hook).
