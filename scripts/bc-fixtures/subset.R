# Byte-code translation cases: `[` and `[[`, the fast forms
# (STARTSUBSET_N, VECSUBSET, MATSUBSET, SUBSET_N) and the default method's
# (STARTSUBSET, DFLTSUBSET), an object's dispatch (scripts/bc-fixtures.R).
cases <- list(
  list(name = "k_vec", fun = function(x, i) x[i],
       calls = c("print(k_vec(c(10, 20, 30), 2))", "print(k_vec(letters, 3:5))",
                 "print(k_vec(c(a = 1, b = 2), 'b'))", "print(k_vec(list(1, 'x'), 2))")),
  list(name = "k_vec2", fun = function(x, i) x[[i]],
       calls = c("print(k_vec2(c(10, 20, 30), 3))", "print(k_vec2(list(a = 1, b = 'x'), 'b'))")),
  list(name = "k_mat", fun = function(m, i, j) m[i, j] + m[[i, j]],
       calls = c("print(k_mat(matrix(1:6, 2), 2, 3))")),
  list(name = "k_subn", fun = function(d, i, j, k) d[i, j, k],
       calls = c("`[.arr3` <- function(x, i, j, k) i * 100 + j * 10 + k",
                 "print(k_subn(structure(list(), class = 'arr3'), 1, 2, 3))")),
  list(name = "k_nested", fun = function(x, i) x[x[i]] + x[[x[[i]]]],
       calls = "print(k_nested(c(3, 1, 2), 2))"),
  list(name = "k_empty", fun = function(m, j) m[, j],
       calls = "print(k_empty(matrix(1:6, 2), 2))"),
  list(name = "k_drop", fun = function(m, i) m[i, , drop = FALSE],
       calls = c("r <- k_drop(matrix(1:6, 2), 1)", "print(dim(r))", "print(c(r))")),
  list(name = "k_exact", fun = function(l) l[["ab", exact = FALSE]],
       calls = "print(k_exact(list(abc = 1, x = 2)))"),
  list(name = "k_factor", fun = function(f, i) f[i],
       calls = "print(k_factor(factor(c('u', 'v', 'u')), 2:3))"),
  list(name = "k_method", fun = function(x, i) c(x[i], x[[i]]),
       calls = c("`[.myc` <- function(x, i) paste('sub', i)",
                 "`[[.myc` <- function(x, i) paste('sub2', i)",
                 "print(k_method(structure(list(), class = 'myc'), 7))")),
  list(name = "k_oob", fun = function(x, i) x[[i]],
       calls = "print(k_oob(1:3, 5))")
)
