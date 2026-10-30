# FastR test snippets

`cases.jsonl` holds the R code of the unit tests of FastR
(https://github.com/oracle/fastr, commit pinned in `../corpora.toml`), taken
from `com.oracle.truffle.r.test/src/com/oracle/truffle/r/test/ExpectedTestOutput.test`
by `scripts/corpus.py import fastr`. Each case id is
`<test class path>/<hash of the code>`; `tests` lists the FastR tests that
contain the snippet.

These snippets are part of FastR, Copyright (c) Oracle and/or its
affiliates, and are distributed under the GNU General Public License
version 3 (https://www.gnu.org/licenses/gpl-3.0.html). They are test data
kept alongside robigo, not part of the robigo program. The expected output
in `expected.jsonl.gz` is recorded from GNU R 4.5.0, not taken from FastR.

`expected-methods.jsonl.gz` records the same 31,374 cases with methods
attached, and is the active expected recording for FastR. It comes from
the oracle recording at 2e2945b, with the 224 changed cases re-recorded
twice in `robigo-oracle:4.5.0` on 2026-10-30.
`methods-changes.tsv` lists every changed case and its paired results.
All 224 cases pass in AST, VM and JIT (224/224), with zero regressions.
Following the merge of environment finalizers (#98), FastR defaults to
attaching `methods` identical to GNU R suites.
