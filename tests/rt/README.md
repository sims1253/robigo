# tests/rt

Conformance cases for G3's runtime, `robigo-rt` (`docs/g3.md`), gated on
GNU R 4.6.2's recorded output (§14.3, L1's own ratchet column).

- A case is `tests/rt/<area>/<name>.R`. Its recording is
  `tests/rt/R-4.6.2/<area>/<name>.{Rout,Rerr,Rstatus,Rsha}`; record it with
  `scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh tests/rt/<area>/<name>.R`.
  These cases are recorded for 4.6.2 only.
- The ratchet is `tests/rt/passing-rt/<area>/<name>.R.pass`, one empty
  marker per case; `robigo-rt`'s ratcheted cases of `tests/r` sit in
  `tests/r/passing-rt/`. `ROBIGO_CONF_UPDATE=1` adds markers; nothing
  removes them.
- The runner: `cargo test --release -p robigo-rt --test conformance --
  --nocapture` (`crates/robigo-conformance`, column `RT`;
  docs/verification.md, "Oracles").

CI runs these through the `rt` job (`scripts/rt-ci.py`): a pull request
that touches only the rt crates and `tests/rt/**` skips the heavy jobs
(`scripts/ci-selection.toml`, rule `rt`; docs/verification.md, CI).
