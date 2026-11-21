# installTrChar in a C locale: a "bytes" name cannot tag a new pairlist
# cell (`x[[name]] <- v` past the end).
x <- pairlist(a = 1)
nm <- "\xe9t"; Encoding(nm) <- "bytes"
r <- tryCatch({ x[[nm]] <- 2; "assigned" }, error = function(e) conditionMessage(e))
print(r)
# a UTF-8 symbol name unserialized in a C locale: installTrChar warns that
# it cannot be translated and installs the escaped name
b <- charToRaw("A\n2\n263682\n131840\n1\n32777\n5\ncaf\\303\\251\n")
r <- withCallingHandlers(as.character(unserialize(b)),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(r)
