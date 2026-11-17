# serialize and unserialize of base's compact sequences (altclasses.c):
# version 3 writes an ALTREP_SXP (the class's info, its state c(n, n1, inc),
# the attributes), whether the sequence has been expanded or not; version 2
# writes the standard vector. unserialize rebuilds the compact sequence.
hex <- function(b) paste(as.character(b), collapse = "")
show <- function(x, ...) {
    for (v in 2:3) for (xdr in c(TRUE, FALSE)) {
        b <- serialize(x, NULL, version = v, xdr = xdr)
        y <- unserialize(b)
        cat(v, xdr, length(b), identical(x, y), identical(b, serialize(y, NULL, version = v, xdr = xdr)), "\n")
    }
    cat(hex(serialize(x, NULL, version = 3)[-(15:23)]), "\n")
    a <- serialize(x, NULL, ascii = TRUE, version = 3)
    cat("ascii", length(a), identical(a, serialize(unserialize(a), NULL, ascii = TRUE, version = 3)), "\n")
}
show(1:10)
show(5:-3)
show(seq_len(4))
show(seq_along(letters))
show(2147483647:2147483650)
show(-0.5:3.5)
show(1:1)
show(integer(0))
# nested, shared, with other objects
show(list(a = 1:3, b = list(seq_len(6)), c = "z"))
s <- 1:20
show(list(s, s))
e <- new.env(); assign("v", 1:7, envir = e)
b <- serialize(e, NULL)
print(get("v", envir = unserialize(b)))
f <- function(n = 1:3) for (i in 1:5) n
show(f)
# an expanded sequence still writes its state
x <- 1:10
invisible(rev(x))
print(sum(x))
show(x)
# attributes make a standard copy (the sequence is not mutable)
show(structure(1:5, foo = "x"))
# a compact sequence with attributes, as only unserialize makes one: the
# ALTREP_SXP item of serialize(1:5) with its last item, the attributes
# (R_NilValue), replaced by the pairlist foo = "x"
be <- function(n) as.raw(c(n %/% 2^24, (n %/% 2^16) %% 256, (n %/% 2^8) %% 256, n %% 256))
b <- serialize(1:5, NULL, version = 3)
n <- length(b)
stopifnot(identical(b[(n - 3L):n], be(254)))
attrs <- c(be(2 + 2^10), be(1), be(9 + 2^18), be(3), as.raw(c(0x66, 0x6f, 0x6f)),
           be(16), be(1), be(9 + 2^18), be(1), as.raw(0x78), be(254))
b <- c(b[-((n - 3L):n)], attrs)
a <- unserialize(b)
print(a)
print(attributes(a))
print(identical(serialize(a, NULL, version = 3), b))
print(identical(serialize(a, NULL, version = 2), serialize(structure(1:5, foo = "x"), NULL, version = 2)))
a2 <- a; a2[1] <- 0L; print(a2); print(a)
print(identical(serialize(a2, NULL, version = 3), b))
