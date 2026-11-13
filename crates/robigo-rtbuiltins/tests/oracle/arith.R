## arithmetic.c: + - * / ^ %% %/% (R_binary, R_unary, do_arith's scalars)
nums <- list(
  NULL0 = NULL, i1 = 5L, i0 = integer(0), iNA = NA_integer_, imax = .Machine$integer.max,
  i3 = c(-7L, 2L, NA), d1 = 2.5, dm = -0.5, d0 = numeric(0), dNA = NA_real_, dNaN = NaN,
  dInf = -Inf, d3 = c(0.1, -3, 1e300), d2 = c(1, 0), l1 = TRUE, l3 = c(TRUE, FALSE, NA))
for (op in c("+", "-", "*", "/", "^", "%%", "%/%")) binary(paste0("b", op), op, nums)
## complex
cx <- list(z1 = 1+2i, zNA = NA_complex_, z2 = c(0+0i, -1.5+0.5i), zN = complex(real = NaN, imaginary = 1),
           zI = complex(real = Inf, imaginary = -2), d1 = 2, i2 = c(3L, NA), l1 = NA, NULL0 = NULL)
for (op in c("+", "-", "*", "/", "^")) binary(paste0("c", op), op, cx)
binary("cmod", "%%", list(z = 1i), list(d = 2, e = numeric(0)))
binary("cidiv", "%/%", list(z = 1i), list(d = 2))
## overflow, precision
case("ovf.plus", quote(x + y), list(x = .Machine$integer.max, y = 1L))
case("ovf.vec", quote(x * y), list(x = c(1L, 100000L, NA, -46341L), y = c(2L, 100000L, 3L, 46341L)))
case("ovf.minus", quote(x - y), list(x = -.Machine$integer.max, y = 1:2))
case("lost", quote(x %% y), list(x = c(1e20, 2, 3e19), y = 3))
case("intmod", quote(x %% y), list(x = c(-7L, 7L, 7L, -7L, 0L), y = c(2L, -2L, 0L, -2L, 3L)))
case("intidiv", quote(x %/% y), list(x = c(-7L, 7L, 7L, NA), y = c(2L, -2L, 0L, 1L)))
case("powint", quote(x ^ y), list(x = c(1L, NA, 2L, -2L, 0L), y = c(NA, 0L, 10L, 3L, -1L)))
case("powreal", quote(x ^ y), list(x = c(-Inf, -Inf, Inf, 0, -8, 1, NaN, NA), y = c(3, 2, -1, -2, 1/3, NA, 0, NaN)))
## recycling
case("recyc.warn", quote(x + y), list(x = 1:3, y = 1:2))
case("recyc.ok", quote(x * y), list(x = 1:6, y = c(10, 20)))
case("recyc.zero", quote(x + y), list(x = 1:3, y = integer(0)))
## NA against NaN in every loop shape
nn <- list(a = c(NA, NaN, 1), b = c(NaN, NA, NA), s = NA_real_, t = NaN, i = c(NA, 1L, 2L), j = NA_integer_, k = 3L)
for (op in c("+", "*", "-", "/", "^")) binary(paste0("nan", op), op, nn)
## attributes
at <- list(
  nm = c(a = 1, b = 2), nm3 = c(x = 1L, y = 2L, z = 3L), foo = structure(1:2, foo = "bar"),
  cls = structure(c(1, 2), class = "foo", extra = TRUE), mat = matrix(1:4, 2, dimnames = list(c("r1", "r2"), NULL)),
  mat2 = matrix(c(1, 2, 3, 4), 2), arr1 = array(5, 1), mat1 = matrix(7), plain2 = c(10, 20), plain3 = 1:3,
  ts3 = structure(c(1, 2, 3), tsp = c(1, 3, 1), class = "ts"), tsp3 = structure(1:3, tsp = c(1, 3, 1)),
  e0 = numeric(0))
for (op in c("+", "/")) binary(paste0("a", op), op, at)
case("a.nonconf", quote(x + y), list(x = matrix(1:4, 2), y = matrix(1:6, 2)))
case("a.ts.short", quote(x + y), list(x = structure(1:2, tsp = c(1, 2, 1)), y = 1:3))
case("a.dim.names", quote(x - y), list(x = matrix(1:2, 1, dimnames = list("r", c("a", "b"))), y = matrix(3:4, 1, dimnames = list("s", NULL))))
case("a.1d", quote(x * y), list(x = array(1:2, 2, list(c("p", "q"))), y = 2L))
case("a.1d.vec", quote(x * y), list(x = array(1:2, 2, list(c("p", "q"))), y = c(u = 1, v = 2)))
case("a.ordering", quote(x + y), list(x = structure(1:2, b = 1, names = c("n", "m"), a = 2), y = structure(1:2, a = 3, c = 4)))
## unary
un <- list(i = c(a = 1L, b = NA), d = structure(c(1.5, -0), dim = 2:1, foo = 1), l = c(p = TRUE, q = NA),
           z = c(1-1i, NA), l1d = array(c(TRUE, FALSE), 2, list(c("u", "v"))), e = logical(0), s = "a", N = NULL, f = list(1))
for (n in names(un)) {
  case(paste0("neg.", n), quote(-x), list(x = un[[n]]))
  case(paste0("pos.", n), quote(+x), list(x = un[[n]]))
}
case("unary.times", quote(`*`(x)), list(x = 2))
case("unary.ztimes", quote(`/`(x)), list(x = 2i))
case("argc0", quote(`+`()))
case("argc3", quote(`+`(x, x, x)), list(x = 1))
## types
case("nonnum.chr", quote(x + y), list(x = "a", y = 1))
case("nonnum.list", quote(x * y), list(x = 1, y = list(1)))
case("nonnum.sym", quote(x - y), list(x = quote(a), y = 1))
## scalar fast paths
case("sc.ii", quote(x / y), list(x = 7L, y = 2L))
case("sc.di", quote(x - y), list(x = 0.5, y = NA_integer_))
case("sc.id", quote(x * y), list(x = NA_integer_, y = NaN))
case("sc.dd", quote(x + y), list(x = NA_real_, y = NaN))
case("sc.dd2", quote(x + y), list(x = NaN, y = NA_real_))
