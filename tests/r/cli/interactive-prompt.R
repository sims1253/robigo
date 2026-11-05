# An interactive console has no prompt under --no-echo (--slave), as
# R_PromptString (main.c) gives: cli's keypress tests drive
# `R -q --slave --vanilla` on a pty and read back only the results.
# The child runs interactively with piped input; lines that only repeat
# the input are dropped (GNU R echoes them there; robigo does not).
exe <- commandArgs()[1]
input <- c("1 + 1", "options(prompt = 'P> ', continue = 'C> ')", "{", "2", "}", "x <- 3", "x", "q('no')")
run <- function(...) {
  out <- system2(exe, c("--interactive", "--no-readline", "--vanilla", "-q", ...),
                 input = input, stdout = TRUE)
  print(out[!out %in% input])
}
run("--no-echo")
run("--slave")
