# %.Nf by exact integer arithmetic (half-even on the binary value, as glibc)
i <- 1:300
x <- c((i * 7919) %% 10007 / 7 * 10^((i %% 13) - 6), -(i / 3), 0.5, 1.5, 2.5, 0.125, 0.375, -0.0001, -0, 0, 1e-320, 5e-324, 999999999999999.9, 0.1 + 0.2, 2^-30, 1/3)
for (p in c(0, 1, 2, 3, 6, 9, 12, 17)) cat(sprintf(paste0("%.", p, "f"), x), sep = "\n")
cat(sprintf("%8.3f|%-10.2f|%+.1f|% .4f|%010.3f", x[1:100], x[2:101], x[3:102], x[4:103], x[5:104]), sep = "\n")
cat(format(x[1:200], nsmall = 2), sep = "\n")
cat(formatC(x[1:200], format = "f", digits = 4), sep = "\n")
cat(round(x[1:200], 3), sep = "\n")
