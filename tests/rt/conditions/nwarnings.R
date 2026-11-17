# options(nwarnings): how many deferred warnings are kept (R_nwarnings);
# setting it drops those pending (R_CollectWarnings = 0); PrintWarnings's
# forms by count.
print(tryCatch(options(nwarnings = 0), error = conditionMessage))
print(getOption("nwarnings"))
f <- function(i) warning(paste("w", i))
g <- function(n) { for (i in seq_len(n)) f(i); invisible() }
g(1)
g(10)
g(11)
g(60)
options(nwarnings = 3)
print(getOption("nwarnings"))
g(5)
print(length(warnings()))
options(nwarnings = 20)
g(15)
g(25)
k <- function() { warning("dropped"); options(nwarnings = 5); warning("kept") }
k()
options(nwarnings = 50)
