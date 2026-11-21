# installTrChar in a C locale: a "bytes" name cannot tag a new pairlist
# cell (`x[[name]] <- v` past the end).
x <- pairlist(a = 1)
nm <- "\xe9t"; Encoding(nm) <- "bytes"
r <- tryCatch({ x[[nm]] <- 2; "assigned" }, error = function(e) conditionMessage(e))
print(r)
