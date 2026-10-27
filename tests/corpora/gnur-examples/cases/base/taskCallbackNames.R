RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: taskCallbackNames
### Title: Query the Names of the Current Internal Top-Level Task Callbacks
### Aliases: getTaskCallbackNames
### Keywords: environment

### ** Examples

 n <- addTaskCallback(function(expr, value, ok, visible) {
                        cat("In handler\n")
                        return(TRUE)
                      }, name = "simpleHandler")

 getTaskCallbackNames()

   # now remove it by name
 removeTaskCallback("simpleHandler")


 h <- taskCallbackManager()
 h$add(function(expr, value, ok, visible) {
                        cat("In handler\n")
                        return(TRUE)
                      }, name = "simpleHandler")
 getTaskCallbackNames()
 removeTaskCallback("R-taskCallbackManager")



