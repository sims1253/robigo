# Removing bindings keeps the order in which the rest are listed
# (ls(sorted = FALSE), names(), as.list(), mget()), in frames large enough
# to be indexed, and after most of a frame was removed and refilled.

# an unhashed frame: bindings listed newest first
e <- new.env(hash = FALSE)
for (i in 1:30) assign(paste0("u", i), i, envir = e)
rm(list = paste0("u", c(1, 5, 17, 30)), envir = e)
ls(e, sorted = FALSE)
for (i in seq(2, 28, by = 2)) rm(list = paste0("u", i), envir = e)
ls(e, sorted = FALSE)
unlist(as.list(e))
assign("u100", 100, envir = e)
assign("u5", 5, envir = e)
ls(e, sorted = FALSE)
unlist(mget(c("u3", "u5", "u100"), envir = e))
exists("u2", envir = e, inherits = FALSE)
get("u29", envir = e)

# a hashed frame: the table's order
h <- new.env(hash = TRUE)
for (i in 1:40) assign(paste0("h", i), i, envir = h)
rm(list = paste0("h", 1:25), envir = h)
ls(h, sorted = FALSE)
for (i in 41:60) assign(paste0("h", i), i, envir = h)
rm(list = paste0("h", c(26, 41, 59)), envir = h)
names(h)
length(h)
sum(unlist(as.list(h)))

# a function frame: its arguments first, in order, then the locals
f <- function(a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14) {
  for (i in 1:20) assign(paste0("l", i), i)
  rm(a3, a7, l1, l2, l3, l20)
  ls(sorted = FALSE)
}
f(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14)

# repeated tags: after removing the first binding the next one is found
g <- eval(quote({
  rm(x)
  r <- x
  rm(x)
  c(r, x)
}), c(structure(as.list(1:20), names = paste0("v", 1:20)),
      list(x = "first", x = "second", x = "third")))
g

# remove everything, one at a time, then reuse the frame
big <- new.env(hash = FALSE)
for (i in 1:200) assign(paste0("b", i), i, envir = big)
for (i in 200:1) rm(list = paste0("b", i), envir = big)
length(big)
ls(big)
assign("z", 1, envir = big)
assign("y", 2, envir = big)
ls(big, sorted = FALSE)
