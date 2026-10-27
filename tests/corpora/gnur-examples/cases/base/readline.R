RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: readline
### Title: Read a Line from the Terminal
### Aliases: readline
### Keywords: utilities

### ** Examples

fun <- function() {
  ANSWER <- readline("Are you a satisfied R user? ")
  ## a better version would check the answer less cursorily, and
  ## perhaps re-prompt
  if (substr(ANSWER, 1, 1) == "n")
    cat("This is impossible.  YOU LIED!\n")
  else
    cat("I knew it.\n")
}
if(interactive()) fun()



