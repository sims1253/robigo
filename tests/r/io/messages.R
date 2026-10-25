message("hello from message")
cat("to stdout\n")
message("a", "b", 1:3)
message("no newline", appendLF = FALSE)
message(" ... continued")
cat("explicit stderr\n", file = stderr())
cat("explicit stdout\n", file = stdout())
writeLines("writeLines to stderr", stderr())
writeLines(c("two", "lines"))
f <- function(x) {
  message("inside f with x = ", x)
  x * 2
}
f(21)
suppressMessages(message("hidden"))
withCallingHandlers(
  message("caught"),
  message = function(m) {
    cat("handler saw:", conditionMessage(m))
    invokeRestart("muffleMessage")
  }
)
res <- tryCatch(message("as condition"), message = function(m) class(m))
res
x <- readline("Your answer: ")
x
nchar(x)
