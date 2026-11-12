# Gap: dput()/deparse() with "warnIncomplete" (part of control = "all")
# warn "deparse may be incomplete" when the output cannot be sourced back
# (R: deparse.c, the 'sourceable' flag). robigo never warns.
f <- eval(parse(text = "function(x) {\n  x\n}", keep.source = TRUE))
attr(f, "srcref") <- NULL
dput(f, control = "all")
dput(new.env(), control = "all")
x <- deparse(quote(f(x)), control = "warnIncomplete")
cl <- quote(g(1))
attr(cl, "a") <- 1
dput(cl, control = "all")
