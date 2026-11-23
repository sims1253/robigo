# gram.y's raiseParseError and raiseLexError: each subclass robigo-syntax
# raises is a classed parseError condition (R_makeErrorCondition(call,
# "parseError", subclass, 4, ...)) with the fields value, filename, lineno
# and colno, its call the parser's offending expression (NULL for the
# lexer's), signalled from inside parse().
show <- function(e) {
  u <- unclass(e)
  cat(paste(class(e), collapse = "/"), "\n")
  for (n in names(u)) cat("  ", n, ": ", deparse(u[[n]]), "\n", sep = "")
}
tryCatch(parse(text = '"\\xg"'), error = show)
tryCatch(parse(text = '"\\u{zz}"'), error = show)
tryCatch(parse(text = '"\\400"'), error = show)
tryCatch(parse(text = 'r"abc"'), error = show)
tryCatch(parse(text = '"\\U{110000}"'), error = show)
tryCatch(parse(text = '"\\u00e9\\xe9"'), error = show)
tryCatch(parse(text = '"\\0"'), error = show)
tryCatch(parse(text = '"\\q"'), error = show)
tryCatch(parse(text = '`\\u00e9`'), error = show)
tryCatch(parse(text = '1 |> _(1)'), error = show)
tryCatch(parse(text = '1 |> f(_)'), error = show)
tryCatch(parse(text = '1 |> f'), error = show)
tryCatch(parse(text = '1 |> f(x = _, y = _)'), error = show)
tryCatch(parse(text = '1 |> function(x) x'), error = show)
tryCatch(parse(text = 'x => y'), error = show)
tryCatch(parse(text = '_ + 1'), error = show)
tryCatch(parse(text = 'function(a, a) 1'), error = show)
# an invalid byte in a UTF-8 session (raiseLexError's invalidMBCS)
tryCatch(parse(text = "zz \xff yy + 1 + 2"), error = show)
# a second line, a tab and a multibyte character before the error: lineno
# is the line, colno counts characters with tabs to the next multiple of 8
tryCatch(parse(text = c("x <- 1", "\t\u00e9; \"\\q\"")), error = show)
