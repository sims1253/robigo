# tests/rt

Conformance cases for G3's runtime, `robigo-rt` (`docs/g3.md`), gated on
GNU R 4.6.2's recorded output (§14.3). The runner (`scripts/rt-conf.py`) and
the ratchet `tests/rt/passing/` arrive with the per-version expected files of
M0; until then the rt crates carry Rust unit tests only.

CI runs these through the `rt` job: a pull request that touches only the rt
crates and `tests/rt/**` skips the heavy jobs (`scripts/ci-selection.toml`,
rule `rt`; `docs/verification.md`, CI).
