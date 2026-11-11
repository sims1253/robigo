# paste() works on CHAR() bytes and marks the result as do_paste does:
# invalid native bytes stay bytes, latin1 is translated, bytes stay bytes.
show <- function(x) { print(Encoding(x)); for (s in x) print(charToRaw(s)) }
a <- "var\xe9"                      # native, not valid UTF-8
show(paste0(a, 1:2))
show(paste(a, "x", sep = "-"))
show(paste(c(a, "b"), collapse = "+"))
l <- "caf\xe9"; Encoding(l) <- "latin1"
show(paste0(l, 1))
show(paste(l, "ü"))
show(paste(l, a))
show(paste(c(l, "x"), collapse = ""))
b <- "\xff\xfe"; Encoding(b) <- "bytes"
show(paste0(b, "x"))
show(paste(b, l))
show(paste(c(b, "y"), collapse = "|"))
print(tryCatch(paste("x", "y", sep = b), error = function(e) conditionMessage(e)))
show(paste0("ü", a))
show(paste(1:2, c("x", "y"), sep = "é"))
# names from paste0() of invalid bytes match latin1 / UTF-8 names (reg-tests-1d)
dvn <- paste0("var\xe9", 1:2); Encoding(dvn) <- "latin1"
u <- "varé2"
print(match(u, dvn)); print(u == dvn)
dv <- data.frame(1:3, 3); names(dv) <- dvn; dv[, u] <- 2
stopifnot(ncol(dv) == 2, dv[, 2] == 2, identical(names(dv), dvn))
