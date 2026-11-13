//! robigo-rtbuiltins: G3's `.Primitive` and `.Internal` layer, ported from
//! GNU R 4.6.2's C sources one module per file (`docs/g3.md` §10), over the
//! safe facade of `robigo-rtcore` (§2.4): `fn(cx, call, args, env, out)`.
//!
//! No `unsafe` here, ever: what a builtin cannot say safely belongs in the
//! facade.
#![forbid(unsafe_code)]

use robigo_rtcore as _;
