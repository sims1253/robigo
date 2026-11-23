# Cstack_info() (R: platform.c:do_Cstack_info) reports the checked stack:
# its size, how much is in use, the direction and the evaluation depth.
# The numbers differ between machines; their properties do not.
x <- Cstack_info()
print(names(x))
print(typeof(x))
print(anyNA(x))
print(x[["size"]] > 0)
print(x[["current"]] > 0 && x[["current"]] < x[["size"]])
print(x[["direction"]])
print(x[["eval_depth"]] > 0)
# Deeper calls are nested deeper.
k <- function(n) if (n == 0) Cstack_info() else k(n - 1)
a <- k(0)
b <- k(50)
print(b[["eval_depth"]] > a[["eval_depth"]] + 40)
print(b[["size"]] == a[["size"]])
# The headroom computation scripts do.
print(x[["size"]] - x[["current"]] > 0)
