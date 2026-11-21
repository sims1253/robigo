# the C locale: a latin1 name installTrChar cannot translate is escaped, with a warning
e <- new.env()
l <- "caf\xe9"; Encoding(l) <- "latin1"
withCallingHandlers(as.name(l), warning = function(w) {print(conditionMessage(w)); invokeRestart("muffleWarning")})
tryCatch(lockBinding(l, e), error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w))
withCallingHandlers(tryCatch(.Internal(remove(l, e, FALSE)), warning = function(w) conditionMessage(w)),
  warning = function(w) print(conditionMessage(w)))
tryCatch(get(l, e), error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w))
