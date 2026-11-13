# Ledgers

One file per track (`docs/workstreams.md`): every experiment, kept or
rejected, with what was tried, why, and the measured result. Newest first.

```markdown
## 2026-10-27: <what>
- why: ...
- result: <metric before -> after>; kept | rejected (<reason>)
```

G3 (`robigo-rt`, `robigo-heap`; `docs/g3.md`) keeps one ledger, `g3.md`,
with dated, append-only sections: its decision bars are written before
the runs they govern (§15.2), and a bar is never edited once a run cites it.
