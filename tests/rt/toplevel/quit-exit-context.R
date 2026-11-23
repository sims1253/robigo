# quit() run by on.exit: .Last reads returnValue() through R_ExitContext,
# which the context reset keeps (and still traces: rt-ci runs this under
# gctorture and the moving collector, which collect inside .Last). A
# gc() call in .Last is left out: after it GNU R reads TRUE, a node-stack
# slot of byte code that ran since, not a value robigo-rt reproduces.
.Last <- function() print(returnValue())
f <- function() { on.exit(quit("no", 3)); list(123) }
f()
