# Details of gram.y's parseError conditions: the lexer's warnings up to the
# failing action's lookahead come before the condition (under warn = 2 the
# first wins); the call keeps its strings' encodings and the native pipe's
# LANGSXP argument cell; a malformed raw literal's position is after its
# delimiter (a newline: the next line at column 0; a tab: column 8).
old <- options(warn = 2)
print(tryCatch(parse(text = "1 |> f(1.1L, _)"), error = function(e) class(e)[1]))
print(tryCatch(str2lang("1 |> f(1.1L, _)"), error = function(e) class(e)[1]))
print(tryCatch(parse(text = "function(a, a = 1.1L) 1"), error = function(e) class(e)[1]))
options(old)
e <- tryCatch(parse(text = '1 |> f("\\xc3\\xa9", _)'), error = identity)
print(Encoding(conditionCall(e)[[2L]]))
e <- tryCatch(parse(text = '1 |> f("\\xe9", _)'), error = identity)
print(Encoding(conditionCall(e)[[2L]]))
e <- tryCatch(parse(text = '1 |> f("\u00e9", _)'), error = identity)
print(Encoding(conditionCall(e)[[2L]]))
e <- tryCatch(parse(text = 'r"'), error = identity)
print(c(e$lineno, e$colno)); print(conditionMessage(e))
e <- tryCatch(parse(text = 'r"\t'), error = identity)
print(c(e$lineno, e$colno))
e <- tryCatch(parse(text = "_ |> f()"), error = identity)
print(identical(serialize(e$call, NULL, version = 2), serialize(quote(f("_")), NULL, version = 2)))
print(typeof(e$call)); print(e$call)
