# Vendored R library sources

These files are copied unmodified from the GNU R 4.5.0 source distribution
(`src/library/base/R`, `src/library/profile`) and are distributed under the
GNU General Public License, version 2 or later (see `COPYING`), like the rest
of rr. Copyright (C) The R Core Team and contributors as noted in each file.

rr implements the `.Primitive`/`.Internal` layer of R in Rust and runs R's
own base-package R code on top of it. Keep these files byte-identical to
upstream; when rr needs a different behaviour, implement the internal
function rather than patching the R code. Any unavoidable local change must
be listed below with a reason.

Local changes: none.

Pinned upstream: R 4.5.0 (2025-04-11), see `scripts/vendor-base.sh`.
