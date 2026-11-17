# Compiled, a scalar double goes to GNU R's MATH1 opcode, which calls the
# C library's function (getMath1Fun) without math1_ari's special points or
# R's small-argument cutoffs; interpreted, do_math1 has them
f <- function(x) 1 / sin(x)
f(-0)
f(-0)
f(-0)
