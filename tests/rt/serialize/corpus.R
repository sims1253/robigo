# serialize(): GNU R's bytes for a generated corpus of values, in each
# format (xdr, binary, ascii, asciihex) and version (2, 3), and
# unserialize() of each gives the value back (L1's bar).
hex <- function(x) paste(as.character(x), collapse = "")
show <- function(name, v) {
  for (ver in c(2L, 3L)) {
    x <- serialize(v, NULL, version = ver)
    cat(name, " v", ver, " xdr ", hex(x), "\n", sep = "")
    cat(name, " v", ver, " bin ", hex(serialize(v, NULL, xdr = FALSE, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " asc ", hex(serialize(v, NULL, ascii = TRUE, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " hex ", hex(serialize(v, NULL, ascii = NA, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " back ", identical(unserialize(x), v), "\n", sep = "")
  }
}
atoms <- list(
  lgl = c(TRUE, NA, FALSE),
  int = c(0L, -1L, NA, 2147483647L, -2147483647L),
  dbl = c(0, -0, 1.5, pi, 1e300, 1e-300, 5e-324, Inf, -Inf, NaN, NA, 1/3, 123456789.125, 1e15, 1e16, -2.5e-7),
  cpl = c(1+2i, NA, complex(real = Inf, imaginary = -0), complex(real = NaN, imaginary = 1)),
  chr = c("", "a", NA, "été", "tab\there", "quote\"s", "back\\slash", "\001ctl", "sp ace", "?'"),
  raw = as.raw(c(0, 1, 32, 127, 128, 255))
)
for (k in names(atoms)) {
  v <- atoms[[k]]
  show(k, v)
  show(paste0(k, "0"), v[0])
  show(paste0(k, "1"), v[1])
  nv <- v
  names(nv) <- paste0("n", seq_along(v))
  show(paste0(k, "-names"), nv)
  av <- structure(v, class = "foo", extra = list(1, "x"))
  show(paste0(k, "-class"), av)
}
show("null", NULL)
show("long-int", rep_len(c(1L, 2L, NA), 3000))
show("matrix", matrix(c(1, 2, 3, 4, 5, 6), 2, 3, dimnames = list(c("a", "b"), NULL)))
show("factor", structure(c(1L, 2L, 1L, NA), levels = c("lo", "hi"), class = "factor"))
show("df", structure(list(x = c(1, 2), y = c("a", "b")), names = c("x", "y"), row.names = c(NA, -2L), class = "data.frame"))
show("list0", list())
show("list", list(1, "a", NULL, TRUE))
show("nested", list(a = list(b = list(c = 1, d = list())), e = "z"))
show("symbol", as.name("a b"))
show("call", quote(f(x, y = 2, ...)))
show("fun-call", quote(function(a, b = 2, ...) a + b))
show("formals", quote(function(a, b = 2) NULL)[[2]])
show("index", quote(x[[1]]$y[-1, drop = FALSE]))
show("if", quote(if (a > 1) b else { c; d }))
show("expr", expression(a, 1 + 2, "s"))
show("missing", quote(x[, 1]))
show("closure", function(x, y = 2) x + y)
show("builtin", sum)
show("special", `if`)
show("prim-list", list(`+`, `[`, length))
# (a closure made by a called function would be byte code: GNU R's JIT
# compiles the function that makes it)
fr <- (function() {
  y <- 1
  z <- "two"
  environment()
})()
g <- eval(quote(function(w) y + w), fr)
show("local-closure", g)
show("emptyenv", emptyenv())
show("globalenv", globalenv())
show("baseenv", baseenv())
show("basens", .BaseNamespaceEnv)
e <- new.env()
for (n in c("b", "a", "c", "zz", "x1", "x2")) assign(n, nchar(n), envir = e)
show("hashed-env", e)
u <- new.env(hash = FALSE)
assign("p", 1, envir = u)
assign("q", "two", envir = u)
show("unhashed-env", u)
show("shared", list(e, e, u, list(e)))
cyc <- new.env()
assign("self", cyc, envir = cyc)
show("cycle", cyc)
lk <- new.env()
assign("v", 1, envir = lk)
lockBinding("v", lk)
lockEnvironment(lk)
show("locked", lk)
ce <- structure(new.env(), class = "myenv")
show("classed-env", ce)
show("env-list", list(f = g, e = environment(g)))
