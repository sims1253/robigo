# Native loops copying elements of character vectors and matrices, as in
# the shootout's naive fasta: `seq[[i]] <- genelist[[2, ind]]`. (Loops run
# natively after 16 iterations, so every case runs longer than that.)

chars <- strsplit("GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGG", "")[[1]]
len <- length(chars)
s2 <- character(len + 20)
for (i in 1:len) s2[[i]] <- chars[[i]]
for (i in 1:20) s2[[len + i]] <- chars[[i]]
print(paste(s2, collapse = ""))

# From a matrix, into a vector; single brackets too.
m <- matrix(c(0.3, "a", 0.2, "c", 0.2, "g", 0.3, "t"), 2)
out <- character(200)
for (i in 1:200) out[[i]] <- m[[2, (i %% 4L) + 1L]]
print(paste(out, collapse = ""))
out2 <- character(50)
for (i in 1:50) out2[i] <- m[2, (i %% 4L) + 1L]
print(paste(out2, collapse = ""))

# Into a matrix.
mm <- matrix("", 2, 30)
for (j in 1:30) {
    mm[[1, j]] <- chars[[j]]
    mm[2, j] <- chars[j + 1]
}
print(mm[, 1:6])

# NA strings are elements like any other.
v <- c("x", NA, "z")
w <- character(30)
for (i in 1:30) w[[i]] <- v[[(i - 1L) %% 3L + 1L]]
print(w)

# In place, reading what was already written.
r <- letters
n <- length(r)
for (i in 1:n) r[[i]] <- r[[n - i + 1L]]
print(r)

# Copy-on-modify: the copy changes, the original does not.
orig <- letters[1:20]
cp <- orig
for (i in 1:20) cp[[i]] <- orig[[21L - i]]
print(rbind(orig, cp))

# Encodings travel with the strings.
u <- rep(c("a", "é", "中"), 10)
uu <- character(30)
for (i in 1:30) uu[[i]] <- u[[i]]
print(uu[1:3])
print(Encoding(uu[1:3]))

# Out of range: writing grows the vector, reading fails.
g <- character(20)
for (i in 1:25) g[[i]] <- chars[[i]]
print(g)
print(tryCatch({
    h <- character(30)
    for (i in 1:30) h[[i]] <- g[[i + 1L]]
    h
}, error = function(e) conditionMessage(e)))
print(tryCatch({
    for (i in 1:30) g[[i]] <- m[[2L + (i > 20L), 1L + (i %% 4L)]]
    g
}, error = function(e) conditionMessage(e)))
print(g)

# An NA index.
idx <- c(1:20, NA, 1:9)
print(tryCatch({
    k <- character(30)
    for (i in 1:30) k[[i]] <- chars[[idx[[i]]]]
    k
}, error = function(e) conditionMessage(e)))
k2 <- character(30)
for (i in 1:30) k2[i] <- chars[idx[i]]
print(k2)

# Attributes: names on the vectors.
nm <- c(a = "A", b = "B", c = "C")
tgt <- c(x = "", y = "", z = "")
for (i in 1:30) tgt[[(i - 1L) %% 3L + 1L]] <- nm[[3L - (i - 1L) %% 3L]]
print(tgt)
for (i in 1:30) tgt[(i - 1L) %% 3L + 1L] <- nm[(i - 1L) %% 3L + 1L]
print(tgt)

# Other types meeting strings.
num <- double(30)
print(tryCatch({
    for (i in 1:30) num[[i]] <- chars[[i]]
    num
}, error = function(e) conditionMessage(e)))
st <- character(30)
for (i in 1:30) st[[i]] <- i * 1.5
print(st)
fa <- factor(rep(c("lo", "hi", "lo"), 10))
fc <- character(30)
for (i in 1:30) fc[[i]] <- as.character(fa[[i]])
print(fc)

# Strings held across an assignment, compared, kept in variables.
sw <- letters
for (i in 1:20) {
    tmp <- sw[[i]]
    sw[[i]] <- sw[[27L - i]]
    sw[[27L - i]] <- tmp
}
print(sw)
cnt <- 0L
for (i in 1:len) if (chars[[i]] == "G") cnt <- cnt + 1L
print(cnt)

# Two vectors written in one loop, from each other.
a1 <- rep("a", 40)
b1 <- rep("b", 40)
for (i in 2:40) {
    a1[[i]] <- b1[[i - 1L]]
    b1[[i]] <- a1[[i - 1L]]
}
print(paste(a1, collapse = ""))
print(paste(b1, collapse = ""))

# In a function, called twice; the vector returned and reused.
copyrev <- function(x) {
    y <- character(length(x))
    n <- length(x)
    for (i in 1:n) y[[i]] <- x[[n - i + 1L]]
    y
}
z1 <- copyrev(chars)
z2 <- copyrev(z1)
print(c(paste(z1, collapse = ""), identical(z2, chars)))

# A generator with <<- in the same loop, as in fastaredux_naive.
rnd <- 42L
nxt <- function() {
    rnd <<- (rnd * 3877L + 29573L) %% 139968L
    rnd / 139968
}
cp4 <- c(0.3, 0.5, 0.7, 1)
sq <- character(600)
for (i in 1:600) {
    r <- nxt()
    lo <- 1L
    while (cp4[[lo]] < r) lo <- lo + 1L
    sq[[i]] <- m[[2, lo]]
}
print(table(sq))
print(rnd)
