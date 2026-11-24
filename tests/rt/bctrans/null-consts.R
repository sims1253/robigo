# Byte code whose constants are NULL (mkCode takes any): LDNULL, RETURN
# needs none, and translates and runs as in GNU R (bcEval reads no
# constant there).
f <- .Internal(bcClose(NULL, .Internal(mkCode(c(12L, 17L, 1L), NULL)), globalenv()))
print(f())
