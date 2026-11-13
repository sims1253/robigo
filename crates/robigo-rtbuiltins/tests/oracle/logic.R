## logic.c: & | ! (do_logic, lbinary, lunary)
vals <- list(NULL0 = NULL, l1 = TRUE, l0 = logical(0), lNA = NA, l3 = c(TRUE, FALSE, NA), i3 = c(0L, 5L, NA),
             d3 = c(0, NaN, -1.5), z2 = c(0i, NA), r2 = as.raw(c(12, 255)), r1 = as.raw(10), s = "a", lst = list(TRUE))
for (op in c("&", "|")) binary(paste0("b", op), op, vals)
at <- list(nm = c(a = TRUE, b = FALSE), mat = matrix(c(TRUE, FALSE, NA, TRUE), 2, dimnames = list(NULL, c("p", "q"))),
           l4 = c(TRUE, TRUE, FALSE, NA), ts = structure(c(TRUE, FALSE), tsp = c(1, 2, 1), foo = 1), e0 = logical(0),
           cls = structure(c(TRUE, NA), class = "foo"))
for (op in c("&", "|")) binary(paste0("a", op), op, at)
case("recyc", quote(x | y), list(x = c(TRUE, FALSE, NA), y = c(FALSE, TRUE)))
case("nonconf", quote(x & y), list(x = matrix(TRUE, 2, 2), y = matrix(TRUE, 1, 4)))
un <- list(l = c(a = TRUE, b = NA), i = structure(c(0L, 3L, NA), dim = 3L, dimnames = list(c("x", "y", "z")), foo = 2),
           d = c(0, NaN, 2), z = c(0i, 1i, NA), r = structure(as.raw(c(0, 15)), foo = "r"), s = "a", s0 = character(0),
           N = NULL, f = list(), lmat = matrix(c(TRUE, FALSE), 1, dimnames = list("r", NULL)), cls = structure(1L, class = "foo"))
for (n in names(un)) case(paste0("not.", n), quote(!x), list(x = un[[n]]))
case("not.scalar", quote(!x), list(x = FALSE))
case("arity1", quote(`&`(x)), list(x = TRUE))
case("arity3", quote(`|`(x, x, x)), list(x = TRUE))
case("arity2not", quote(`!`(x, x)), list(x = TRUE))
