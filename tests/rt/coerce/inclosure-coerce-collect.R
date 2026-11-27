# Remaining in-closure coerce() callers moved onto coerce_vector / fill_* (B (iv)):
# match5, duplicated(incomparables), pmin/pmax, nzchar, names<- (pairlist),
# class<- (ascommon), and vector("pairlist", n), long enough to collect inside
# one call under gctorture, young:16, and retry-torture.

# 1. match() coercing numeric -> character (match5)
m <- match(1:500, c("1", "250", "500"))
cat("match:", length(m), m[1], m[2], m[250], m[500], "\n")
m2 <- match(1:300, c("1", "100", "300"), incomparables = c(100L))
cat("match-incomp:", m2[1], is.na(m2[100]), m2[300], "\n")

# 2. duplicated() / unique() with incomparables coerced to character
dv <- paste0(c(1:200, 1:50))
du <- duplicated(dv, incomparables = 1:100)
cat("dup-incomp:", sum(du), du[201], du[250], "\n")

# 3. pmin / pmax coercing numeric -> character
pm <- pmin("200", 1:400)
px <- pmax("200", 1:400)
cat("pmin-pmax:", length(pm), pm[1], pm[300], px[1], px[300], "\n")

# 4. nzchar() coercing numeric -> character
nz <- nzchar(c(1:400, NA_integer_), keepNA = TRUE)
cat("nzchar:", length(nz), sum(nz, na.rm = TRUE), is.na(nz[401]), "\n")

# 5. names<- from a pairlist of scalars coerced to character
x <- integer(300)
pl <- as.pairlist(as.list(1:300))
names(x) <- pl
cat("names-pl:", length(names(x)), names(x)[1], names(x)[300], "\n")

# 6. class<- coercing through ascommon (character & list)
y <- 1:400
class(y) <- "character"
cat("class-chr:", typeof(y), length(y), y[1], y[400], "\n")
z <- 1:300
class(z) <- "list"
cat("class-lst:", typeof(z), length(z), z[[1]], z[[300]], "\n")

# 7. vector("pairlist", n) through Cx::fill_pairlist
vp <- vector("pairlist", 300)
cat("vec-pl:", typeof(vp), length(vp), is.null(vp[[1]]), is.null(vp[[300]]), "\n")
