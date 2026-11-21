# sys-unix.c's Sys.info (do_sysinfo). Its values differ per machine, so only
# the names, the type and the shape are printed.
x <- Sys.info()
typeof(x)
length(x)
names(x)
names(attributes(x))
all(nzchar(x))
!anyNA(x)
x[["sysname"]]
identical(x, .Internal(Sys.info()))
tryCatch(.Internal(Sys.info(1)), error = function(e) conditionMessage(e))
