# Character subscripts against names: first match wins, "" and NA never
# match, and both the linear and the hashed lookups (GNU R hashes above a
# size threshold) agree. Lookups interleaved with updates must see every
# change of the names, whichever way the names were changed.

x <- c(a = 1, b = 2, a = 3, 4, c = 5)
names(x)[4] <- ""
x["a"]; x[["a"]]
x[c("a", "c", "a")]
x[""]; x[NA_character_]
x[c("", NA)]
y <- x; y[["a"]] <- 10; y
y <- x; y["a"] <- 20; y
y <- x; y[[""]] <- 30; y
y <- x; y[c("z", "z", "a", "")] <- 1:4; y
y <- x; y[[NA_character_]] <- 40; y
names(y)
z <- c(1, 2); names(z) <- c("q", NA); z[[NA_character_]] <- 3; z
tryCatch(x[["zz"]], error = function(e) conditionMessage(e))
tryCatch(x[[""]], error = function(e) conditionMessage(e))

# lists: exact and partial matching
l <- list(alpha = 1, beta = 2, alpha = 3, al = 4)
l[["alpha"]]; l$alpha; l$be; l[["be", exact = FALSE]]
l$al; l[["alp", exact = FALSE]]
op <- options(warnPartialMatchDollar = TRUE)
l$bet
options(op)
l[["al"]] <- NULL; names(l)
l[["gamma"]] <- 5; names(l)

# large vectors take the hashed path in GNU R
n <- 3000L
big <- seq_len(n)
nm <- paste0("k", big %% 1700L)
nm[c(5L, 10L)] <- ""
nm[7L] <- NA
names(big) <- nm
big["k5"]; big[["k5"]]
big[c("k1", "k1699", "k0", "", NA, "k5")]
big[["k1699"]] <- 0L; big[c(1699L, 3399L)]
s <- paste0("k", 1:1500)
r <- big[s]
sum(r, na.rm = TRUE); sum(is.na(r)); names(r)[1500]
big2 <- big; big2[c("new1", "k3", "new1", "new2")] <- -(1:4)
length(big2); tail2 <- big2[(n - 1L):(n + 2L)]; tail2
big2[["k3"]]; big["k3"]

# many subscripts, few names
sm <- c(u = 1, v = 2, u = 3)
ss <- rep(c("u", "v", "w", ""), 400)
table(sm[ss], useNA = "ifany")
sm2 <- sm; sm2[ss] <- seq_along(ss); length(sm2); sm2[1:7]; table(names(sm2))

# knucleotide-style counting: lookups interleaved with growth
count <- function(keys) {
    f <- integer(0)
    for (k in keys) {
        cnt <- f[k]
        if (is.na(cnt)) cnt <- 0L
        f[[k]] <- cnt + 1L
    }
    f
}
keys <- c(letters, letters[1:5], "zz", letters[3:10])
cnt <- count(keys)
cnt[order(names(cnt))][1:12]; length(cnt)
keys2 <- paste0("w", (seq_len(2500) * 7L) %% 1300L)
cnt2 <- count(keys2)
length(cnt2); sum(cnt2); cnt2[c("w0", "w7", "w1299", "w650")]

# growth with padding (the knucleotide_brute_3 pattern)
f <- integer(4); cap <- 4L; n <- 0L
for (k in c("x", "y", "x", "z", "w", "v", "y", "u")) {
    cnt <- f[k]
    if (is.na(cnt)) {
        cnt <- 0L
        n <- n + 1L
        f[[cap <- if (cap < n) 2L * cap else cap]] <- 0L
    }
    f[[k]] <- cnt + 1L
}
f

# the names change by other routes between lookups
v <- c(p = 1, q = 2, r = 3)
v[["q"]]
names(v)[2] <- "s"; v["q"]; v[["s"]]
names(v) <- c("q", "q", "t"); v[["q"]]; v["t"]
attr(v, "names") <- c("m", "n", "o"); v["n"]
w <- v; names(w)[1] <- "n"; w[["n"]]; v[["n"]]
v[["o"]] <- 30; length(v) <- 5; v; v["o"]
names(v)[5] <- "o"; v[["o"]]; v["o"]
v <- v[c("o", "m")]; v; v[["m"]]
v <- c(v, o = 99); v[["o"]]; v["o"]
v <- rev(v); v[["o"]]
big3 <- big; names(big3)[1] <- "k1699"; big3[["k1699"]]; big3["k1699"]
names(big3) <- rev(names(big3)); big3[["k1"]]; big3["k1"]
e <- c(a = 1); e[["a"]] <- 2; e2 <- e; e2[["b"]] <- 3; e; e2; e[["b"]] <- 4; e; e2
