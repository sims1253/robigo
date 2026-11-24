# A name given as a string whose escapes take it past MAXIDSIZE (LC_ALL=C,
# the .locale sidecar): installTrChar's translateToNative warns, then
# install fails ("variable names are limited to 10000 bytes"), interpreted
# and in a closure the JIT compiles (cmp.R's addVar or as.name warns,
# then fails: the closure runs uncompiled).
nm <- paste(rep("\u4e2d", 1251), collapse = "")
options(warn = 1)
tryCatch(eval(substitute(missing(N), list(N = nm))), error = conditionMessage)
tryCatch(eval(substitute(N <- 1, list(N = nm))), error = conditionMessage)
fl <- eval(substitute(function() { N <- 1; for (i in 1) NULL; 0 }, list(N = nm)))
for (i in 1:2) print(tryCatch(fl(), error = conditionMessage))
# `<<-` makes no local: cmpAssign's as.name warns, then fails, so the closure
# is not compiled and warns again at each call
fs <- eval(substitute(function() { N <<- 1; for (i in 1) NULL; 0 }, list(N = nm)))
for (i in 1:2) print(tryCatch(fs(), error = conditionMessage))
options(warn = 2)
tryCatch(eval(substitute(missing(N), list(N = nm))), error = conditionMessage)
