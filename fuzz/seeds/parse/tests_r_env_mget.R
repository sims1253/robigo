show <- function(expr)
    tryCatch({
        x <- eval.parent(substitute(expr))
        print(x)
    }, error = function(e) cat("ERR:", conditionMessage(e), "\n"))
a <- 1
b <- "2"
# R: envir.c:do_mget — mode is vectorised over x.
show(mget(c("a", "b"), mode = c("numeric", "character")))
show(mget(c("a", "b"), mode = "bogus"))
show(mget(character(0), mode = "bogus"))
# NA names are looked up as "NA"; only an empty *first* name is invalid.
show(mget(NA_character_))
show(mget(c("a", "")))
show(mget(c("", "a")))
show(mget("a", ifnotfound = globalenv()))
show(mget("a", ifnotfound = function(x) paste0("<", x, ">")))
show(mget("a", mode = NA_character_))
show(mget(c("a", ""), mode = c("any", "bogus")))
