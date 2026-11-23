# scientific(): long double scaling as GNU R (tbl[] holds double literals)
x <- c(1/3, 2/3, 1e-15, 1.234567890123456e-20, 9.9999999999999999e24, 123456789012, 1e300,
  5e-324, 0.1 + 0.2, 100, 1e5, 1e15, 1e16, 123456.7, -0.000012345, pi * 1e27, pi * 1e-28,
  2^70, 99999.5, 0.99999999, 0x1.f9c10830a7fa6p-38)
print(x)
as.character(x)
format(x, digits = 3)
for (d in c(1, 3, 7, 10, 15, 16, 17, 22)) print(format(c(pi, exp(1) * 1e-30, 1/7 * 1e40), digits = d))
signif(2/3, 1:15)
format(9996, digits = 3)
format(c(9.9996e24, 9.9996e26), digits = 3)
print(c(1e-300, 1e-310), digits = 15)
i <- 1:400
z <- (i^1.7 * 10^((i %% 70) - 35)) / 7
as.character(z)
as.character(round(z * 10^(i %% 9), i %% 6) / 10^(i %% 9))
