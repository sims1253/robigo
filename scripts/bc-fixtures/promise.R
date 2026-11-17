# Byte-code translation cases: a promise compiled code makes holds its own
# code (MAKEPROM's BCODESXP, its PRCODE); what reads its expression reads
# the code's source (PREXPR, bytecodeExpr): substitute(), missing(),
# match.call()'s `...`, an unused argument's message; a builtin called
# through CALL runs its argument's code (bcEval), a complex assignment's
# getter and setter calls likewise (scripts/bc-fixtures.R).
cases <- list(
  list(name = "k_subst", fun = function(x) h(x + 1, x, 2),
       calls = c("h <- function(y, z, w) list(substitute(y), substitute(z), substitute(w), y, z)",
                 "print(k_subst(2))")),
  list(name = "k_missing", fun = function(a) g(a),
       calls = c("g <- function(b) missing(b)", "print(k_missing())", "print(k_missing(1))")),
  list(name = "k_missing2", fun = function(a, b) g2(a, b + 1),
       calls = c("g2 <- function(p, q) c(missing(p), missing(q))", "print(k_missing2())",
                 "print(k_missing2(1, 2))")),
  list(name = "k_matchcall", fun = function(x) m(x + 1, 'a', k = x),
       calls = c("n <- function(...) match.call()", "m <- function(...) n(...)", "print(k_matchcall(1))")),
  list(name = "k_unused", fun = function(x) u(x + 1, x * 2),
       calls = c("u <- function(a) a", "print(tryCatch(k_unused(1), error = conditionMessage))")),
  list(name = "k_force", fun = function(x, y) p(x + y, sin(x), y),
       calls = c("p <- function(a, b, c) c(a, b, c)", "print(k_force(1, 2))")),
  list(name = "k_dots", fun = function(x) d(x + 1, x * 3),
       calls = c("d <- function(...) list(..., substitute(list(...)))", "print(k_dots(2))")),
  list(name = "k_builtin", fun = function(f, x) f(x + 1, 2, x),
       calls = c("print(k_builtin(max, 5))", "print(k_builtin(c, 1L))")),
  list(name = "k_inline", fun = function(n) { s <- 0; for (i in 1:n) s <- s + sq(i + 1); s },
       calls = c("sq <- function(a) a * a", "print(k_inline(10L))", "print(k_inline(3L))")),
  list(name = "k_setter", fun = function(x) { tag(x, i + 1) <- 5; x },
       calls = c("i <- 1", "`tag<-` <- function(x, j, value) { attr(x, 'j') <- deparse(substitute(j)); x[j] <- value; x }",
                 "print(k_setter(c(1, 2, 3)))")),
  list(name = "k_getter", fun = function(x, nm) { attr(x, nm)[2] <- 9; x },
       calls = "print(k_getter(structure(1, a = c(1, 2)), 'a'))"),
  # an unforced promise is written with its code: MAKEPROM's byte code
  list(name = "k_serialize", fun = function(x) sr(x + 1),
       calls = c("sr <- function(y) { r <- serialize(environment(), NULL, xdr = FALSE); c(length(r), sum(as.integer(r))) }",
                 "print(k_serialize(1))")),
  # and read back, it runs that code
  list(name = "k_unserialize", fun = function(x) sr2(x + 1),
       calls = c("sr2 <- function(y) { e <- unserialize(serialize(environment(), NULL)); eval(quote(y), e) }",
                 "print(k_unserialize(1))"))
)
