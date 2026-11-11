# options(warn = 2) turns the lexer's unpaired-surrogate warning into an
# error; with show.error.messages off, verrorcall_dflt prints nothing and
# the script halts before the statement runs.
options(warn = 2, show.error.messages = FALSE)
cat("before\n")
x <- "\uD800"
cat("after\n")
