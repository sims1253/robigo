# the C locale: a latin1 name installTrChar cannot translate is escaped, with a warning
e <- new.env()
l <- "caf\xe9"; Encoding(l) <- "latin1"
withCallingHandlers(as.name(l), warning = function(w) {print(conditionMessage(w)); invokeRestart("muffleWarning")})
tryCatch(lockBinding(l, e), error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w))
withCallingHandlers(tryCatch(.Internal(remove(l, e, FALSE)), warning = function(w) conditionMessage(w)),
  warning = function(w) print(conditionMessage(w)))
tryCatch(get(l, e), error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w))
# translateToNative's warning comes before install's length error
x <- paste0(strrep("a", 10000), "\xe9"); Encoding(x) <- "latin1"; Encoding(x)
tryCatch(as.name(x), warning = function(w) conditionMessage(w), error = function(e) conditionMessage(e))
withCallingHandlers(tryCatch(as.name(x), error = function(e) conditionMessage(e)),
  warning = function(w) {print(substr(conditionMessage(w), 1, 30)); invokeRestart("muffleWarning")})
op <- options(warn = 2); tryCatch(as.name(x), error = function(e) substr(conditionMessage(e), 1, 40)); options(op)
