# The partialMatchWarning of attr() under options(warnPartialMatchAttr =
# TRUE) (attrib.c:do_attr): a partial match of another attribute, of
# "names" (only when there are names), none when exact = TRUE or the
# option is off.
x <- structure(1, abcdef = 2)
y <- c(a = 1)
print(attr(x, "abc"))
options(warnPartialMatchAttr = TRUE)
h <- function(w) {
  print(class(w)); print(w$argument); print(w$formal); print(conditionCall(w))
  invokeRestart("muffleWarning")
}
r <- withCallingHandlers(attr(x, "abc"), warning = h); print(r)
r <- withCallingHandlers(attr(y, "nam"), warning = h); print(r)
r <- withCallingHandlers(attr(x, "nam"), warning = h); print(r)
r <- withCallingHandlers(attr(x, "abc", exact = TRUE), warning = h); print(r)
print(getOption("warnPartialMatchAttr"))
options(warnPartialMatchAttr = FALSE)
r <- withCallingHandlers(attr(x, "abc"), warning = h); print(r)
