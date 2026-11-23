# seq() from the global environment after registerS3method("seq", "default"):
# GNU R's dispatch finds the registered method in base's table. robigo's VM
# reads ranges off as long as the table holds any seq.default (gap).
f <- function() {
    s <- 0L
    for (j in seq(1L, 3L, 1L)) s <- s + j
    for (j in seq(length.out = 3L)) s <- s + j
    s
}
print(f())
registerS3method("seq", "default", function(...) 99L, envir = baseenv())
print(f())
print(seq(1L, 3L, 1L))
