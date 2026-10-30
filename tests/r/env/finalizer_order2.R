events <- character()
add <- function(x) events <<- c(events, x)

# three finalizers on one key run in reverse registration order
e <- new.env()
reg.finalizer(e, function(e) add("first"))
reg.finalizer(e, function(e) add("second"))
reg.finalizer(e, function(e) add("third"))
rm(e)
invisible(gc())
cat("same key:", paste(events, collapse = ","), "\n")

# three dead keys run in reverse registration order
events <- character()
a <- new.env()
b <- new.env()
c <- new.env()
reg.finalizer(a, function(e) add("a"))
reg.finalizer(b, function(e) add("b"))
reg.finalizer(c, function(e) add("c"))
rm(a, b, c)
invisible(gc())
cat("keys:", paste(events, collapse = ","), "\n")

# only the dead keys run, in one batch
events <- character()
a <- new.env()
b <- new.env()
c <- new.env()
reg.finalizer(a, function(e) add("a"))
reg.finalizer(b, function(e) add("b"))
reg.finalizer(c, function(e) add("c"))
rm(b)
invisible(gc())
cat("mixed:", paste(events, collapse = ","), "\n")
rm(a, c)
invisible(gc())
cat("mixed dead:", paste(events, collapse = ","), "\n")

# a parent chain dies from the child outwards
events <- character()
parent <- new.env()
child <- new.env(parent = parent)
parent$id <- "parent"
child$id <- "child"
reg.finalizer(child, function(e) add("child"))
reg.finalizer(parent, function(e) add("parent"))
rm(parent)
invisible(gc())
cat("chain child:", paste(events, collapse = ","), "\n")
rm(child)
invisible(gc())
cat("chain dead:", paste(events, collapse = ","), "\n")
