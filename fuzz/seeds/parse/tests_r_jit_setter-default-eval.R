# The same uncaught error inside anonymous eval/parse.
eval(parse(text = "{ x <- 1; x[[]] <- 42; x }"))
