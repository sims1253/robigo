# check_stack_balance and loops: a top-level loop runs compiled (only the
# loop itself checks); in a local frame the AST runs it, and `break` and
# `next` leave by a longjmp to the loop's context, which restores the
# protect stack.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
mark("for"); for (i in 1:2) .Call("leak")
mark("while"); while (TRUE) { .Call("leak"); break }
mark("repeat"); repeat { .Call("leak"); break }
mark("local-for"); local(for (i in 1:2) .Call("leak"))
mark("local-next"); local({ for (i in 1:2) { .Call("leak"); next } })
mark("local-break"); local({ repeat { .Call("leak"); break } })
mark("local-while"); local(while (TRUE) { .Call("leak"); break })
mark("end")
