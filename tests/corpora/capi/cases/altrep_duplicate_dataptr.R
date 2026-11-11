dyn.load("altrep_lazy_string.so")
lazy <- function(fn) .Call("lazy_new", fn)

# The class's Duplicate method returns NULL, so GNU R copies the vector
# itself, through the writeable DATAPTR the class refuses
# (altrep.c, memory.c:duplicate1). The materialization runs before, with the
# target bound (#550).
l <- lazy(function() { print(typeof(l)); c("u", "w") })
print(tryCatch({ l$p <- 1; "no error" }, error = function(e) conditionMessage(e)))
a <- list(b = lazy(function() { print(names(a)); c("u", "w") }))
print(tryCatch({ a$b$p <- 1; "no error" }, error = function(e) conditionMessage(e)))
print(names(a))
