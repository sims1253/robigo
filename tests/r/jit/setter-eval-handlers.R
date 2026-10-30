# Condition calls resolve the marker, even when default output would retain it.
for (src in c("x <- 1; x[[]] <- 42", "x <- 1; x[[NA]] <- 7",
              "f <- function(b, i) { b[i] <- 1 }; f(f, 3)",
              "x <- matrix(0, 1, 1); x[[2, 1]] <- 7")) {
    tryCatch(eval(parse(text = src)), error = function(e) {
        print(conditionCall(e))
        print(conditionMessage(e))
    })
}
