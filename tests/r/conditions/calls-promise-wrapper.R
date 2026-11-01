# A simple .Internal wrapper (identical is one of compiler's
# safeBaseInternals) evaluated while forcing a promise keeps its closure
# frame in the error 'Calls:' trace: GNU R never compiles promise
# expressions, so forcing evaluates the AST and calls the wrapper's
# closure (issue #157).

cat(identical(1$foo, 2), "\n")
