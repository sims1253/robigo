# The persistent-name hooks: serialize's refhook is called at each
# encounter of an environment (not the special ones), and unserialize's
# restores what it names.
hex <- function(x) paste(as.character(x), collapse = "")
e <- new.env()
assign("k", 1, envir = e)
names_seen <- character()
hook <- function(x) {
  cat("hook", class(x), "\n")
  if (identical(x, e)) "the-env" else NULL
}
x <- serialize(list(e, e, globalenv(), new.env()), NULL, refhook = hook)
cat(hex(x), "\n")
back <- unserialize(x, refhook = function(s) {
  cat("restore", s, "\n")
  e
})
print(identical(back[[1]], e))
print(identical(back[[2]], e))
print(identical(back[[3]], globalenv()))
print(tryCatch(unserialize(x), error = function(e) conditionMessage(e)))
y <- serialize(list(e, 1), NULL, refhook = function(x) NULL)
print(identical(unserialize(y)[[2]], 1))
