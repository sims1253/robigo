# R: objects.c:do_usemethod (translateChar(STRING_ELT(generic, 0))),
# subset.c:R_subset3_dflt (translateChar(input)). robigo-rt copies the
# generic's and the `$` name's bytes to the stack for a short ASCII name
# and to the heap past 64 bytes; a name that needs translation is
# translated first. Each kind once, with UseMethod and with `$` on a list
# and an environment, by symbol and by string.
show <- function(expr) print(tryCatch(expr, error = function(e) conditionMessage(e)))
long <- strrep("g", 70L)
u <- "café"
l <- "caf\xe9"
Encoding(l) <- "latin1"
cat(Encoding(u), Encoding(l), nchar(long), "\n")
x <- structure(1, class = "k")
assign(paste0(long, ".k"), function(x) "the long name's method")
assign(paste0(u, ".k"), function(x) "the UTF-8 name's method")
f_long <- function(x) UseMethod(long)
f_u <- function(x) UseMethod(u)
f_l <- function(x) UseMethod(l)
show(f_long(x))
show(f_u(x))
show(f_l(x))
show(f_l(structure(1, class = "none")))
lst <- list(1, 2, 3, 4)
names(lst) <- c(long, u, "ab", "abc")
e <- new.env()
assign(long, "long in e", envir = e)
assign(u, "u in e", envir = e)
for (nm in list(long, u, l, "ab", strrep("g", 66L))) {
    show(eval(as.call(list(as.name("$"), quote(lst), nm))))
    show(eval(as.call(list(as.name("$"), quote(lst), as.name(nm)))))
    show(eval(as.call(list(as.name("$"), quote(e), nm))))
}
# install's checks on an environment's name (installTrChar)
show(do.call("$", list(e, "")))
show(do.call("$", list(e, strrep("a", 10001L))))
show(do.call("$", list(e, strrep("a", 10000L))))
