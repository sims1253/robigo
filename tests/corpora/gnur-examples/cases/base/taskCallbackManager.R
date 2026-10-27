### Name: taskCallbackManager
### Title: Create an R-level Task Callback Manager
### Aliases: taskCallbackManager
### Keywords: environment

### ** Examples

# create the manager
h <- taskCallbackManager()

# add a callback
h$add(function(expr, value, ok, visible) {
                       cat("In handler\n")
                       return(TRUE)
                     }, name = "simpleHandler")

# look at the internal callbacks.
getTaskCallbackNames()

# look at the R-level callbacks
names(h$callbacks())

removeTaskCallback("R-taskCallbackManager")



