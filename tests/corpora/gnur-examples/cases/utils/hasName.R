### Name: hasName
### Title: Check for Name
### Aliases: hasName
### Keywords: manip logic

### ** Examples

x <- list(abc = 1, def = 2)
!is.null(x$abc) # correct
!is.null(x$a)   # this is the wrong test!
hasName(x, "abc")
hasName(x, "a")



