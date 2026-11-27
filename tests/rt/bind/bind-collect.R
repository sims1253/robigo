# c() and unlist() collecting inside their per-element loops (Cx::fill_strings
# for StringAnswer and NewExtractNames, Cx::fill_list for ListAnswer on VECSXP
# and EXPRSXP): long enough to collect inside one call under gctorture and the
# young-moving collector, with no unrooted fresh list/nm/base objects.

# 1. StringAnswer (c and unlist, coercing atomic leaves across multiple args)
s1 <- c("head", (1:1500) / 3, 1:500)
cat("s1:", length(s1), s1[1], s1[2], s1[1501], s1[1502], s1[2001], "\n")
s2 <- unlist(list("u0", list((1:1000) / 7, c(TRUE, NA, FALSE)), 1:500))
cat("s2:", length(s2), s2[1], s2[2], s2[1001], s2[1002], s2[1003], s2[1504], "\n")

# 2. ListAnswer (VECSXP and EXPRSXP, recursive FALSE and TRUE, scalars per elt)
l1 <- c(list("first"), (1:1500) / 3)
cat("l1:", typeof(l1), length(l1), l1[[1]], l1[[2]], l1[[1501]], "\n")
e1 <- c(expression(x + 1), 1:1500)
cat("e1:", typeof(e1), length(e1), deparse(e1[[1]]), e1[[2]], e1[[1501]], "\n")
l2 <- c(list(a = 1:500, b = list(c = 501:1000)), expression(z), recursive = FALSE)
cat("l2:", typeof(l2), length(l2), names(l2), length(l2[[1]]), deparse(l2[[3]]), "\n")
l3 <- unlist(list(list(1:500), list(501:1000, quote(sym))), recursive = TRUE)
cat("l3:", typeof(l3), length(l3), l3[[1]], l3[[500]], l3[[1000]], deparse(l3[[1001]]), "\n")

# 3. NewExtractNames (fill_strings for names, nested prefixes, count==1 vs >1,
#    UTF-8 base/tag, combined with fill_list and fill_strings)
n1 <- c(a = 1:1500, b = 1L, c = list(d = 1:500, e = 99L), recursive = TRUE)
nm1 <- names(n1)
cat("n1:", length(n1), nm1[1], nm1[1500], nm1[1501], nm1[1502], nm1[2001], nm1[2002], "\n")
n2 <- unlist(list(p = list(q = 1:1000, r = 1L), s = c(x = 1, y = 2)))
nm2 <- names(n2)
cat("n2:", length(n2), nm2[1], nm2[1000], nm2[1001], nm2[1002], nm2[1003], "\n")
n3 <- c(a = list(1), b = 1:1000)
nm3 <- names(n3)
cat("n3:", typeof(n3), length(n3), nm3[1], nm3[2], nm3[1001], "\n")
n4 <- unlist(list(a = 1:500, b = 501:1000), recursive = FALSE)
cat("n4:", typeof(n4), length(n4), names(n4)[1], names(n4)[500], names(n4)[501], names(n4)[1000], "\n")

# 4. UTF-8 nested base and count == 1 encoding preservation
u <- intToUtf8(c(99L, 97L, 102L, 233L))
x <- list(1L)
names(x) <- u
r1 <- c(x, 2L)
cat("u1:", names(r1), Encoding(names(r1)), "\n")
y <- list(list(1L, 2L))
names(y) <- u
r2 <- unlist(y)
cat("u2:", names(r2), Encoding(names(r2)), "\n")
