# Errors from gram.y's raiseParseError/raiseLexError are classed
# parseError conditions with value, filename, lineno and colno fields;
# plain grammar errors stay simpleErrors.
show <- function(e) {
  cat("class:", class(e), "\n")
  u <- unclass(e)
  for (n in names(u)) cat(" ", n, "=", deparse(u[[n]]), "\n")
}
r <- tryCatch(parse(text = '1 |> f(_)'), error = show)
r <- tryCatch(parse(text = 'x y'), error = show)
r <- tryCatch(parse(text = '"\\x"'), error = show)
r <- tryCatch(parse(text = '"\\400"'), error = show)
r <- tryCatch(parse(text = '"\\q"'), error = show)
r <- tryCatch(parse(text = 'function(a, a) 1'), error = show)
r <- tryCatch(parse(text = 'f(`\\u00e9`)'), error = show)
r <- tryCatch(parse(text = '1 |> function(x) x'), error = show)
r <- tryCatch(parse(text = '1 |> 2'), error = show)
r <- tryCatch(parse(text = 'f(_)'), error = show)
r <- tryCatch(parse(text = 'x => y'), error = show)
r <- tryCatch(parse(text = '1 |> f(_, _)'), error = show)
r <- tryCatch(parse(text = '_(1) |> f()'), error = show)
r <- tryCatch(parse(text = '"\\U{110000}"'), error = show)
r <- tryCatch(parse(text = '"\\u"'), error = show)
r <- tryCatch(str2lang('1 |> g(_)'), error = show)
r <- tryCatch(str2expression('"\\0"'), error = show)
# A calling handler for the class runs before the default error.
r <- try(withCallingHandlers(parse(text = '1 |> f(_)'),
  parseError = function(e) cat("calling:", class(e)[1], "\n")), silent = TRUE)
cat(r)
