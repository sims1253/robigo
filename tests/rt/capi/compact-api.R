# Package C handing base's compact sequences to the C API: setAttrib's
# dim, tsp and row.names read the value through INTEGER or REAL, which
# expands it as GNU R's Dataptr does; lengthgets copies it. (These reached
# a slice read unexpanded: AC1's builtins expand their arguments at entry,
# and C's setAttrib is no builtin.)
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/compact_api.so"))
d <- 2:3
print(.Call("rt_ca_expanded", d))
v <- .Call("rt_ca_dim", 6L, d)
print(dim(v))
print(c(v))
print(.Call("rt_ca_expanded", d))
# tsp: start 1, end 2, frequency 3 fits 4 observations
tp <- 1:3
w <- .Call("rt_ca_tsp", 4L, tp)
print(attr(w, "tsp"))
print(.Call("rt_ca_expanded", tp))
# 1:n as row names is stored as c(NA, -n), and read back as 1:n
rn <- 1:3
l <- .Call("rt_ca_rownames", rn)
print(attr(l, "row.names"))
print(.Call("rt_ca_expanded", rn))
rn2 <- 3:1
l2 <- .Call("rt_ca_rownames", rn2)
# kept as they are, read by element (print would expand them in robigo-rt
# and identical in GNU R: the AC1 ledger's divergences)
a <- attr(l2, "row.names")
print(c(length(a), a[[1L]], a[[3L]]))
# shortRowNames reads INTEGER(s)[0] only of an integer vector of length 2
print(.row_names_info(l2, 1L))
print(.Call("rt_ca_expanded", rn2))
rn3 <- 3:4
l3 <- .Call("rt_ca_rownames", rn3)
print(.Call("rt_ca_expanded", rn3))
print(.row_names_info(l3, 1L))
print(.Call("rt_ca_expanded", rn3))
x <- 1:3
print(.Call("rt_ca_lengthgets", x, 5L))
print(.Call("rt_ca_lengthgets", 5:4, 1L))
print(.Call("rt_ca_lengthgets", x, 3L))
print(.Call("rt_ca_expanded", x))
# to length 0 nothing is copied, so nothing expands
z <- 1:4
print(.Call("rt_ca_lengthgets", z, 0L))
print(.Call("rt_ca_expanded", z))
# a factor whose codes are a compact sequence, as dimnames: asCharacterFactor
# reads the codes with INTEGER_ELT
codes <- 1:2
m <- .Call("rt_ca_factor_dimnames", codes, c("a", "b"))
print(dimnames(m))
print(.Call("rt_ca_expanded", codes))
