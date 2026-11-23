# GNU R's compact integer/real sequences (altclasses.c) serialize in ALTREP
# form; copies and modified vectors do not.
n <- function(x) length(serialize(x, NULL))
n(1:10); n(1:1e6); n(10:1); n(seq_len(5)); n(seq_along(letters))
n(5:5); n(c(1:3)); n(rev(1:5)); n(1:5 + 0L)
n(as.numeric(1:5)); n(as.numeric(5:1)); n(as.integer(as.numeric(1:5)))
n(3e9:(3e9 + 4))
x <- 1:5; y <- x; y[2] <- 9L; n(x); n(y)
z <- 1:4; names(z) <- letters[1:4]; n(z)
n(serialize(1:3, NULL, version = 2))
s <- serialize(1:3, NULL)
s[1:60]
unserialize(s)
identical(unserialize(serialize(1:5, NULL)), 1:5)
u <- unserialize(serialize(-2:2, NULL)); u; n(u)
n(.row_names_info(data.frame(a = 1:3), 0L))
