# The two checksums both sides of the lazyLoad comparison compute
# (crates/robigo-rtapi/tests/lazyload.rs includes this file; scripts/
# rt-rdb-roundtrip.sh's GNU R side sources a copy beside the data): one copy
# of the contract, so the sides cannot drift apart.
# ck: the serialization's length and a position-weighted sum of its bytes
# (version 2, after the header), in doubles: exact up to 2^53.
ck <- function(x) { b <- as.numeric(as.integer(serialize(x, NULL, version = 2L))[-(1:14)])
                    paste(length(b), sum(b * seq_along(b)) %% 1000000007) }
# nz: x with every closure in it (itself, in a list or an environment)
# replaced by its formals, body and attributes, and the top-level
# environment by its sorted contents (nested ones by a mark): closures' and
# environments' enclosures aside.
nz <- function(x, top = TRUE) if (is.function(x) && !is.primitive(x)) list(formals(x), body(x), attributes(x)) else
                  if (is.environment(x)) { if (top) list("env", nz(as.list(x, all.names = TRUE, sorted = TRUE), FALSE)) else "env" } else
                  if (is.list(x)) { a <- attributes(x); r <- lapply(x, nz, top = top); attributes(r) <- a; r } else x
