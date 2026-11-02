# gram.c reads the byte after the backslash, so the error position falls
# inside a multibyte character.
r <- tryCatch(parse(text = "x <- \"\\é\""), error = function(e) conditionMessage(e))
cat(grepl("unrecognized escape", r, useBytes = TRUE), "\n")
cat(sub(".*\\(", "(", r, useBytes = TRUE), "\n")
r <- tryCatch(parse(text = "\"é\\é\""), error = function(e) conditionMessage(e))
cat(sub(".*\\(", "(", r, useBytes = TRUE), "\n")
cat("survived\n")
