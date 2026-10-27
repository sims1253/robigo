### Name: showConnections
### Title: Display Connections
### Aliases: showConnections getConnection getAllConnections
###   closeAllConnections stdin stdout stderr nullfile isatty
### Keywords: connection

### ** Examples

showConnections(all = TRUE)
## Not run: 
##D textConnection(letters)
##D # oops, I forgot to record that one
##D showConnections()
##D #  class     description      mode text   isopen   can read can write
##D #3 "letters" "textConnection" "r"  "text" "opened" "yes"    "no"
##D mycon <- getConnection(3)
## End(Not run)

c(isatty(stdin()), isatty(stdout()), isatty(stderr()))



