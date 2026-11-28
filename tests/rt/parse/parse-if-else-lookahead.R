# parse(con, n = 1) after an action error at a newline in an if body (a
# gram.y 'i' context, outside the top level): yylex reads past the newline
# to the next token to see whether it is `else`, so that token, and what it
# looked at past it, is read (#1058). Not at the top level, not inside a
# bracket opened in the body, not after `else`.
rem <- function(txt) {
  f <- tempfile(); writeLines(txt, f, sep = "", useBytes = TRUE); con <- file(f, "r")
  r <- tryCatch({parse(con, n = 1); "ok"}, error = function(e) class(e)[1])
  cat(deparse(txt), r, "|", deparse(readLines(con, warn = FALSE)), "\n"); close(con)
}
bodies <- c("1 |> 2", "f(_)", "1 |> f(y = _)", "x |> _$a")
tails <- c("\nfoo bar\nNEXT\n", "\n\n# c\n  foo\nNEXT\n", "\n}\nNEXT\n", "\n)\nNEXT\n", "\nelse 3}\nNEXT\n", "\n12 + 1\nNEXT\n")
heads <- c("{if(TRUE) ", "{if(a) if(b) ", "{function() if(a) ", "(if(a) ", "x[if(a) ", "if(a) ", "{if(a) 1 else ", "{if(a) {", "{for(i in 1) if(a) ", "f(if(a) ")
for (h in heads) for (b in bodies) for (t in tails) rem(paste0(h, b, t))
rem("{if(a) 1 |> 2, foo\nNEXT\n")
rem("f(if(a) 1 |> 2, foo)\nNEXT\n")
rem("{if(a) 1 |> 2; foo\nNEXT\n")
