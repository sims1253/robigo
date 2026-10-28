# GNU R's src/appl (C part)

`optim.c` (with `lbfgsb.c`), `uncmin.c` and `integrate.c` from GNU R
4.5.0's `src/appl`, unmodified (GPL-2 | GPL-3, like `rlib/`). GNU R
builds them into libR and exports `vmmin`, `nmmin`, `cgmin`, `samin`,
`lbfgsb`, `optif9`, `fdhess`, `Rdqags` and `Rdqagi` (R_ext/Applic.h);
`build.rs` compiles them into robigo's libR the same way, against
`../include` plus the two shims here (`Defn.h`: the translation macro
`optim.c` takes from GNU R's internal header; `config.h`: empty).

The Fortran LINPACK routines they call (`dpofa`, `dtrsl`) and the rest
of `src/appl/*.f` are Rust ports in `src/linpack.rs`.
