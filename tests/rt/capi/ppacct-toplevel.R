# eval.c's check_stack_balance at the top level: a routine that leaves an
# object protected (support/ppleak.c) gets "stack imbalance" from every
# builtin and special around it that the AST interpreter evaluates, each
# with GNU R's protect-stack depths before and after. A statement starts
# with the stack back where the REPL left it.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
mark("toplevel"); .Call("leak")
mark("invisible"); invisible(.Call("leak"))
mark("c1"); invisible(c(.Call("leak")))
mark("c2"); invisible(c(1, .Call("leak")))
mark("plus"); invisible(1 + .Call("leak"))
mark("paren"); invisible((.Call("leak")))
mark("brace"); { .Call("leak") }
mark("two"); { .Call("leak"); .Call("leak") }
mark("sibling"); { .Call("leak"); invisible(1) }
mark("if"); if (TRUE) .Call("leak")
mark("ifcond"); if (is.null(.Call("leak"))) 1
mark("andand"); is.null(.Call("leak")) && TRUE
mark("assign"); x <- .Call("leak")
mark("assign-brace"); x <- { .Call("leak") }
x <- list()
mark("dollar-assign"); x$y <- .Call("leak")
mark("names-assign"); names(x) <- .Call("leak")
mark("subset"); c(1)[.Call("leak")]
mark("switch"); switch("a", a = .Call("leak"))
mark("internal"); invisible(.Internal(inherits(.Call("leak"), "a", FALSE)))
mark("balanced"); invisible(.Call("noop"))
mark("end")
