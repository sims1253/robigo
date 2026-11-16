## 2026-11-16: the conformance-runner and oracle-pin deferrals

- What: the foreman's package of deferred review items from #644, #638,
  #669, #680 and #648.
- Tree: branch `g3/conformance-pins` on main `dd932c78` (#694 merged).

1. **One list of the oracle pins** (#644). `scripts/oracle-versions` holds
   `gated=4.5.0` and `rt=4.6.2`. Its header states the format: LF lines
   (`.gitattributes`), each empty, a comment (`#` first) or `NAME=X.Y.Z`
   (lowercase letters, three runs of digits, no spaces), each `NAME` once.
   The readers: crates/robigo-conformance at compile time (`include_str!`
   and a `const fn`, `pin_in`, that checks the whole file: a malformed file,
   a missing pin or a name given twice does not build, and the panic names
   the pin it was reading), `scripts/corpus.py` (`pins()`, the same checks),
   and `scripts/regen-expected.sh`, `scripts/ci-image.sh` and
   `scripts/base-subset.sh`, which source it; `scripts/test_corpus.py`
   checks that the shell reads the same values (OpenCodeReview's six
   threads on #696). Each had its own copy with a
   "keep in step" comment. `.github/workflows/ci.yml` is unchanged: its
   names for the rt oracle image's output (`image_oracle_4_6_2`) and the
   sources' cache keys (`r-4.6.2-tarball`, `r-4.5.0-sources`) stay literal,
   and `scripts/test_corpus.py` (CI's `lint` job runs it) fails when
   they name anything but the pins, so a version bump changes the file and
   the test names what else to edit. `scripts/ci-image.sh refs` prints the
   same tags as main's script (the pins file is not in any image's
   context). `scripts/r-src.sh` keeps its own `version=4.6.2`: it pins
   robigo-rt's source input (L4), not an oracle.
2. **The rt conformance runner** (#638 r4): done in `cfcb2f0c` ("one runner
   for both runtimes"), with tests in `crates/robigo-conformance/tests/runner.rs`:
   `ROBIGO_R_VERSION` is read with `var_os` and a non-UTF-8 value is an
   error, never the gated version; another version's partial
   `.Rout`/`.Rerr`/`.Rstatus`/`.Rsha` set is "partly recorded" and not
   compared; a non-gated run that compared nothing fails.
3. **`rt-bars-compare.py`'s `allocs()`** (#669 r3): more than one
   `*allocs.json` for a run is an error, as `load()`'s duplicate row is.
4. **`base-subset.sh`'s row names** (#680): two rows of
   `benchmarks/rt/calls-rows.toml` with one name are an error (any group:
   each row becomes the script of its name).
5. **`record.sh`'s grep** (#648; the file is
   `crates/robigo-rtbuiltins/tests/oracle/record.sh`): `grep -Ec ' = | E : '`
   for GNU grep's `\|` in a basic regular expression; the same counts on
   every recorded `.out`.

### Proof

- `python3 scripts/test_corpus.py` (the new pin tests among its 19);
  `cargo test --release -p robigo-conformance`; the rt conformance reports
  "against GNU R 4.6.2" through the new constant; `regen-expected.sh`
  refuses 4.4.0 and asks for 4.5.0 by default, 4.6.2 with
  `ROBIGO_R_VERSION`.
- The two duplicate guards, on fixtures: two allocs files, two rows named
  alike, each a loud error; the real rows still generate.
