probe <- function(label, expression) {
    cat(label, "\n")
    tryCatch({
        y <- eval(expression)
        if (is.character(y)) {
            print(Encoding(y))
            print(lapply(y, charToRaw))
        } else print(y)
    }, error=function(e) cat("ERROR:", conditionMessage(e), "\n"))
}
x <- rawToChar(as.raw(233))
Encoding(x) <- "latin1"
probe("marked input, implicit source", quote(iconv(x, to="UTF-8", toRaw=TRUE)))
probe("marked input, explicit source", quote(iconv(x, from="latin1", to="UTF-8", toRaw=TRUE)))
probe("marked input, explicit UTF-8", quote(iconv(x, from="UTF-8", to="UTF-8", toRaw=TRUE)))
probe("UTF-8 to marked latin1", quote(iconv("é", from="UTF-8", to="latin1")))
probe("UTF-8 to unmarked latin1", quote(iconv("é", from="UTF-8", to="latin1", mark=FALSE)))
probe("unmarked UTF-8 output", quote(iconv("é", from="UTF-8", to="UTF-8", mark=FALSE)))
probe("raw latin1 to string", quote(iconv(list(as.raw(233)), from="latin1", to="latin1")))
probe("marked substitution bytes", quote(iconv(x, from="latin1", to="ASCII", sub="byte")))
