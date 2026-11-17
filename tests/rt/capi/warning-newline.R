# errors.c's warning() drops one trailing newline of a message C code
# formats; warningcall() does not. C in support/warnings.c, built by
# scripts/rt-capi-build.sh into RT_CAPI_LIBS.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/warnings.so"))
h <- function(expr) withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w))
    print(conditionCall(w))
    invokeRestart("muffleWarning")
})
f <- function(m) .Call("capi_warning", m)
h(f("one trailing newline\n"))
h(f("two trailing newlines\n\n"))
h(f("no newline"))
h(f("\n"))
h(.Call("capi_warningcall", "kept by warningcall\n"))
f("at top level\n")
