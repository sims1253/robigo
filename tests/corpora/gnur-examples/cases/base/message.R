RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: message
### Title: Diagnostic Messages
### Aliases: message suppressMessages packageStartupMessage
###   suppressPackageStartupMessages .makeMessage
### Keywords: programming

### ** Examples

message("ABC", "DEF")
suppressMessages(message("ABC"))

testit <- function() {
  message("testing package startup messages")
  packageStartupMessage("initializing ...", appendLF = FALSE)
  Sys.sleep(1)
  packageStartupMessage(" done")
}

testit()
suppressPackageStartupMessages(testit())
suppressMessages(testit())



