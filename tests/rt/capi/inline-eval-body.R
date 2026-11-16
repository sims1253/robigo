# A body written in place by package C (docs/g3.md §9.3) after its calls ran
# inline: a closure GNU R's JIT never compiles runs its AST, which reads
# each statement from the live body, so the next call runs the new
# statement. robigo's inline run of its `Eval` code gives up at the
# statement boundary (the code is stale), and the real call resumes the
# rest as GNU R's AST does.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
mk <- function() { bal <- 0; function(x) { bal <<- bal + x; invisible(bal) } }
d <- mk()
run <- function() { for (i in 1:3) r <- d(1); r }
for (k in 1:3) print(run())
invisible(.Call("rt_set_body_elt", d, 1L, quote(bal <<- bal + 100 * x)))
for (k in 1:3) print(run())
invisible(.Call("rt_set_body_elt", d, 2L, quote(invisible(-bal))))
for (k in 1:3) print(run())
