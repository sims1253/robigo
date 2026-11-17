# `<<-` from a loop body into a generic's argument that its method's alias
# reads, alongside another `<<-` in the same body. robigo-rt runs such a
# body inline and logs the writes; a write into an aliased argument makes
# the generic's promise first, so the inline run gives the write back to the
# real call (#744). Either way, the method keeps the generic's promise and
# the generic sees its new bindings.
gen <- function(x) {
  y <- 0
  on.exit(print(c(x, y)))
  hook <<- function(v) x <<- v * 2
  hook2 <<- function(v) y <<- v * 3
  UseMethod("gen")
}
gen.default <- function(x) {
  for (i in 1:20) {
    hook(i + 0.5)
    hook2(i + 0.25)
  }
  x
}
print(gen(8))
gen2 <- function(x) {
  on.exit(print(x))
  hook <<- function(v) x <<- v + 0.5
  UseMethod("gen2")
}
gen2.default <- function(x) {
  s <- 0
  for (i in 1:5) s <- s + hook(i)
  c(x, s)
}
print(gen2(2))
