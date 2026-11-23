# R_ParseVector from package C (support/eval.c): a grammar error and an
# incomplete input come back as the status (PARSE_ERROR 3, PARSE_INCOMPLETE
# 2), as yyerror leaves them; gram.y's classed parse errors are signalled
# from inside R_Parse (raiseParseError, raiseLexError ->
# R_signalErrorCondition) and never return to the C caller, so R code
# around the .Call catches the parseError.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/eval.so"))
print(.Call("capi_parse_status", "1 + 2"))
print(.Call("capi_parse_status", "x y"))
print(.Call("capi_parse_status", "f(1,"))
r <- tryCatch(.Call("capi_parse_status", "function(a, a) 1"), error = function(e) class(e))
print(r)
r <- tryCatch(.Call("capi_parse_status", "\"\\q\""), parseError = function(e) conditionMessage(e))
print(r)
