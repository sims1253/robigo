# GNU R's mapply reuses one call whose dots[[j]][[k]] indices it updates in
# place, so a call captured by FUN shows the last index once mapply is done
g <- function(x, y) sys.call()
print(mapply(g, 1:2, 3:4, SIMPLIFY = FALSE))
