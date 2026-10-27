library(parallel)
### Name: sendData
### Title: Cluster Back-end Interface
### Aliases: closeNode recvData recvOneData sendData
### Keywords: internal

### ** Examples
## Not run: 
##D   # A toy cluster consisting of one connection.
##D   sendData.mynode <- function(node, data) serialize(data, node)
##D   recvData.mynode <- function(node) unserialize(node)
##D   recvOneData.mycluster <- function(cl) list(
##D     node = 1, value = recvData(cl[[1]])
##D   )
##D   closeNode.mynode <- function(node) close(node)
##D 
##D   # Not shown: R starting a serverSocket on the other end, ready to
##D   # accept connections and evaluate jobs
##D   cl <- structure(list(
##D     structure(
##D       socketConnection(..., blocking = TRUE, open = 'a+b'),
##D       class = 'mynode'
##D     )
##D   ), class = c('mycluster', 'cluster'))
##D   clusterEvalQ(cl, Sys.getpid())
##D   stopCluster(cl)
##D   rm(cl)
## End(Not run)


