# compilerOptions' reads take their values from their own frame, not from an
# R function of the session: a base list() rebound to return NULL in that
# frame changes nothing (it made robigo-rt panic).
env <- baseenv()
unlockBinding("list", env)
orig <- base::list
assign("list", function(...) {
    p <- parent.frame()
    if (exists("optimize", p, inherits = FALSE) && exists("suppressAll", p, inherits = FALSE)) NULL else orig(...)
}, envir = env)
f <- function() { for (i in 1:2) NULL; 7 }
print(f())
print(f())
assign("list", orig, envir = env)
