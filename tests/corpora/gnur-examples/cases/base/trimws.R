RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: trimws
### Title: Remove Leading/Trailing Whitespace
### Aliases: trimws
### Keywords: character

### ** Examples

x <- "  Some text. "
x
trimws(x)
trimws(x, "l")
trimws(x, "r")

## Unicode --> need "stronger" 'whitespace' to match all :
tt <- "text with unicode 'non breakable space'."
xu <- paste(" \t\v", tt, "\u00a0 \n\r")
(tu <- trimws(xu, whitespace = "[\\h\\v]"))
stopifnot(identical(tu, tt))



