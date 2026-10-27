### Name: globalVariables
### Title: Declarations Used in Checking a Package
### Aliases: globalVariables suppressForeignCheck
### Keywords: programming

### ** Examples

## Not run: 
##D ## assume your package has some code that assigns ".obj1" and ".obj2"
##D ## but not in a way that codetools can find.
##D ## In the same source file (to remind you that you did it) add:
##D if(getRversion() >= "2.15.1")  utils::globalVariables(c(".obj1", "obj2"))
##D 
##D ## To suppress messages about a run-time calculated native symbol, 
##D ## save it to a local variable.
##D 
##D ## At top level, put this:
##D if(getRversion() >= "3.1.0") utils::suppressForeignCheck("localvariable")
##D 
##D ## Within your function, do the call like this:
##D localvariable <- if (condition) entry1 else entry2
##D .Call(localvariable, 1, 2, 3)
##D 
##D ## HOWEVER, it is much better practice to write code
##D ## that can be checked thoroughly, e.g.
##D if(condition) .Call(entry1, 1, 2, 3) else .Call(entry2, 1, 2, 3)
## End(Not run)



