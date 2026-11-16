# check_stack_balance on the AST's live statement list: a callback writes
# the statement a suspended `{` runs next (support/closure_mutation.c), so
# the rest of the `{` is do_begin's on the live list (robigo-rt's
# Op::Resume deopt), with do_begin's depths and the `{`'s own check.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
cb <- function(f) .Call("rt_callback", f)
# a body GNU R never compiles (it may call browser()): always the AST
nb <- function() {
  if (FALSE) browser()
  cb(function() .Call("rt_set_body_elt", nb, 3L, quote(invisible(.Call("leak")))))
  "old statement"
}
mark("resumed"); nb()
mark("resumed-again"); nb()
mark("end")
