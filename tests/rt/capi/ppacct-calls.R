# check_stack_balance under base's byte-compiled functions: their own
# protections count (lapply's, do.call's, eval's ...), their code checks
# nothing, a special .Internal (lapply, vapply) checks in do_internal.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
mark("lapply"); invisible(lapply(1, function(i) .Call("leak")))
mark("lapply2"); invisible(lapply(1:2, function(i) .Call("leak")))
mark("vapply"); invisible(vapply(1:2, function(i) { .Call("leak"); 1 }, 1))
mark("local"); local(.Call("leak"))
mark("eval"); eval(quote(.Call("leak")))
mark("eval-env"); eval(quote(.Call("leak")), new.env())
mark("do.call"); do.call(".Call", list("leak"))
mark("do.call-closure"); do.call(function() .Call("leak"), list())
mark("tryCatch"); tryCatch(.Call("leak"), error = function(e) NULL)
mark("tryCatch-error"); tryCatch({ .Call("leak"); stop("e") }, error = function(e) NULL)
mark("withCallingHandlers"); withCallingHandlers(.Call("leak"))
mark("end")
