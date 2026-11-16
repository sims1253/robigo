# The link-walk family (link-walk.R): paste stores an object's
# as.character() value back into its argument list (SET_VECTOR_ELT), so a
# later method that writes a value an earlier one returned copies it:
# "a Q", not "Q Q".
z <- c("a", "b")
as.character.foo <- function(x, ...) {
  if (unclass(x) == 2) z[1] <<- "Q"
  z
}
p <- structure(1, class = "foo")
q <- structure(2, class = "foo")
print(paste(p, q))
print(z)
z <- c("a", "b")
print(paste0(p, q, collapse = "+"))
