# coerceVector of an atomic vector to character (coerceToString), made with
# collections between elements (Cx::fill_strings): long enough to collect
# inside one call under gctorture, with each kind of element, and the
# attributes the callers keep or drop (as.character and as.vector clear
# them; storage.mode<- and names<- go through coerceVector, which keeps
# them; dimnames<- clears each component's). The values are GNU R's.
x <- as.character((1:2000) / 3)
length(x); x[c(1, 2, 1999, 2000)]; sum(nchar(x))
as.character(c(TRUE, NA, FALSE)); as.character(c(1L, NA, -3L))
as.character(c(1.5, NA, NaN, Inf, -Inf, 1e300, 1e-300, 0.1 + 0.2))
as.character(c(1+2i, NA, -0.5i)); as.character(as.raw(c(0, 15, 255)))
y <- as.character(seq_len(2000) * 1i)
y[c(1, 2000)]
z <- structure(1:6, dim = 2:3, names = letters[1:6], class = "foo", extra = "e")
attributes(as.character(z)); attributes(as.vector(z, "character"))
storage.mode(z) <- "character"
attributes(z); unclass(z); is.object(z)
v <- 1:4
names(v) <- c(10, 20, 30, 40)
names(v)
m <- matrix(1:4, 2)
dimnames(m) <- list(1:2, c(1.5, 2.5))
dimnames(m)
as.character(integer(0)); as.vector(numeric(0), "character")
# the other callers of coerceVector to character, long enough to collect
# inside the call: storage.mode<- (attributes kept), dimnames<- (each
# component's attributes cleared); and vapply's widening of a result
# (logical to double: it never makes character), through coerceVector too
w <- structure((1:2000) / 7, names = paste0("n", 1:2000), tag = "t")
storage.mode(w) <- "character"
w[c(1, 2000)]; names(w)[2000]; attr(w, "tag")
mm <- matrix(0, 2000, 1)
dimnames(mm) <- list(structure((1:2000) / 9, extra = "x"), "c")
rownames(mm)[c(1, 2000)]; attributes(rownames(mm))
vv <- vapply(c(TRUE, NA, FALSE), function(x) x, 1)
vv; typeof(vv)
