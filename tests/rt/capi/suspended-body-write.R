# A callback writing a later statement of the closure that called it, while
# that closure is suspended in the callback (docs/g3.md §9.3: "a suspended
# compiled activation whose assumption broke deopts at its next
# uncommitted operation"). GNU R walks an AST body live (do_begin reads the
# next cell after each statement), so the AST runs the new statement; byte
# code (past the JIT threshold) runs the old one. robigo-rt has no deopt of
# a suspended activation yet, so this case is not run (its .requires:
# suspended-deopt).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
cb <- function(f) .Call("rt_callback", f)
# a body GNU R never compiles (it may call browser()): always the AST
nb <- function() {
  if (FALSE) browser()
  cb(function() .Call("rt_set_body_elt", nb, 3L, "new statement"))
  "old statement"
}
print(nb())
print(nb())
# a small closure: its first call is the AST, the second compiles the body
# the first call's callback wrote
s <- function() {
  cb(function() .Call("rt_set_body_elt", s, 2L, "new statement"))
  "old statement"
}
print(s())
invisible(.Call("rt_set_body_elt", s, 2L, "old statement"))
print(s())
print(s())
