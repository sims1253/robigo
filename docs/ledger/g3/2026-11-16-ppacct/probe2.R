dyn.load(Sys.getenv("PPLEAK"))
mark <- function(s) cat("##", s, "\n", file = stderr())
mark("two"); { .Call("leak"); .Call("leak") }
mark("sibling"); { .Call("leak"); invisible(1) }
mark("next"); local({ for (i in 1:2) { .Call("leak"); next } })
mark("break"); local({ repeat { .Call("leak"); break } })
mark("while"); local(while (TRUE) { .Call("leak"); break })
mark("topwhile"); while (TRUE) { .Call("leak"); break }
mark("toprepeat"); repeat { .Call("leak"); break }
fr <- function() { .Call("leak"); return(1) }
mark("return-erase"); invisible(fr())
mark("return-erase2"); invisible(fr())
mark("vapply"); invisible(vapply(1:2, function(i) { .Call("leak"); 1 }, 1))
mark("ifcond"); if (is.null(.Call("leak"))) 1
mark("andand"); is.null(.Call("leak")) && TRUE
mark("switch"); switch("a", a = .Call("leak"))
x <- list()
mark("dollar-assign"); x$y <- .Call("leak")
mark("names-assign"); names(x) <- .Call("leak")
mark("subset"); c(1)[.Call("leak")]
mark("wch"); withCallingHandlers(.Call("leak"))
f <- function(x) x
mark("ff"); f(f(.Call("leak")))
mark("err"); tryCatch({ .Call("leak"); stop("e") }, error = function(e) NULL)
mark("evalenv"); eval(quote(.Call("leak")), new.env())
mark("anon"); (function() .Call("leak"))()
mark("internal"); invisible(.Internal(inherits(.Call("leak"), "a", FALSE)))
g2 <- function(x) UseMethod("g2"); g2.a <- function(x) NextMethod(); g2.default <- function(x) .Call("leak")
mark("nextmethod"); g2(structure(1, class = "a"))
length.foo <- function(x) { .Call("leak"); 1L }
mark("internal-generic"); length(structure(1, class = "foo"))
"+.foo" <- function(e1, e2) .Call("leak")
mark("ops"); structure(1, class = "foo") + 1
mark("docall-closure"); do.call(function() .Call("leak"), list())
mark("end")
