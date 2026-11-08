# needs: rlang
# An unhandled rlang error at top level: the process must print the
# message and backtrace and exit 1, in seconds. The failure mode this
# pins is a hang in the backtrace formatting (state "timeout").
suppressPackageStartupMessages(library(rlang))
inner <- function() abort("smoke boom", use_cli_format = TRUE)
outer <- function() inner()
outer()
cat("unreachable\n")
