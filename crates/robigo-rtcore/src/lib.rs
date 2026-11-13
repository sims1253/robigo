//! robigo-rtcore: the facade of G3's runtime over `robigo-heap`
//! (`docs/g3.md` §2): the isolate capability `Cx`, branded handles
//! (`Ref<'g>`, `View<'g>`), roots (`Root<'s>`, `Out<'s>`, `Args<'a>`) and
//! the value stack.
//!
//! This is the one crate of the runtime whose `unsafe` the layers above
//! rely on; every block is audited here so that `robigo-rtbuiltins` can
//! forbid it. Dependency direction (§14.1): heap → rtcore → rtbuiltins →
//! rt → rtapi → cli.
//!
//! Empty until R1 fills it (`docs/ledger/g3.md`). It takes its dependency on
//! `robigo-heap` when that crate lands.
