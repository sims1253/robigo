# Ledgers

One file per track (`docs/workstreams.md`): every experiment, kept or
rejected, with what was tried, why, and the measured result. Newest first.

```markdown
## 2026-10-27: <what>
- why: ...
- result: <metric before -> after>; kept | rejected (<reason>)
```

G3 (`robigo-rt`, `robigo-heap`; `docs/g3.md`) keeps its ledger in
`g3.md` (entries up to 2026-11-13) and `g3/`, one file per entry
(`YYYY-MM-DD-<topic>.md`), so parallel pull requests never conflict. Its
decision bars are written before the runs they govern (§15.2), and a bar
is never edited once a run cites it.
