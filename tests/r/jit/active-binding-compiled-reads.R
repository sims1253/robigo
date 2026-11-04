# How often an active binding's function runs when compiled code reads it.
# GNU R's bytecode (GETVAR on an active binding) runs it twice per read
# once the closure is compiled; the AST interpreter once.
cnt <- 0
makeActiveBinding("act", function() { cnt <<- cnt + 1; 3 }, globalenv())
ra <- function() act * 2
x <- double(5)
for (i in 1:5) x[[i]] <- ra()
print(cnt)
cnt <- 0
for (i in 1:5) ra()
print(cnt)
cnt <- 0
ra(); print(cnt)
