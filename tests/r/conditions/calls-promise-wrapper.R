# A simple .Internal wrapper (identical is one of compiler's
# safeBaseInternals) evaluated while forcing a promise keeps its closure
# frame in the error 'Calls:' trace: a promise made by top-level code
# (the AST evaluator) is not compiled, so forcing evaluates the AST and
# calls the wrapper's closure (issue #157). Promises compiled code makes
# are bytecode and do inline (compiler/simple-internals-contexts.R).

cat(identical(1$foo, 2), "\n")
