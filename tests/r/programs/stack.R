new_stack <- function() {
  items <- list()
  push <- function(x) {
    items[[length(items) + 1]] <<- x
    invisible(NULL)
  }
  pop <- function() {
    top <- items[[length(items)]]
    items[[length(items)]] <<- NULL
    top
  }
  size <- function() length(items)
  list(push = push, pop = pop, size = size)
}
s <- new_stack()
s$push(1)
s$push("two")
s$push(3)
s$size()
s$pop()
s$pop()
s$size()
balanced <- function(str) {
  chars <- strsplit(str, "")[[1]]
  depth <- 0
  for (ch in chars) {
    if (ch == "(") depth <- depth + 1
    if (ch == ")") depth <- depth - 1
    if (depth < 0) return(FALSE)
  }
  depth == 0
}
balanced("(a(b)c)")
balanced("(()")
balanced(")(")
