# Ledger: core (robigo-core, robigo-capi; goalpost G2)

Track: `docs/workstreams.md` § core; design: `docs/core.md`. Metrics: (a)
`capi` corpus cases passing, (b) weighted census coverage by implemented
`libR.so` exports, (c) the gate packages `fastmap`, `digest`.

Local setup (this machine): system R is 4.6.1, so the capi corpus's
`R CMD SHLIB` build step runs GNU R 4.5.0 from the `robigo-oracle:4.5.0`
image through wrapper scripts (`~/.cache/robigo-core/bin/{R,Rscript}`:
`docker run --rm -u $(id -u):$(id -g) -v $HOME:$HOME -w $PWD
robigo-oracle:4.5.0 R "$@"`) put first on `PATH`; robigo itself runs on
the host with `R_HOME=~/.local/opt/R-4.5.0`.

## 2026-10-28: baseline

- capi corpus: 0/20 pass (stdout 0, stderr 0, status 2 match): `dyn.load`
  registers no code, `.Call` fails with "native routine ... is not
  implemented yet".
- census coverage by robigo exports: 0 / 5339 (no `libR.so`).
