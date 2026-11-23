options(warn = 2)
x <- c(1L, 2L)
a <- tracemem(x)
print(identical(tracemem(x), a))
print(withVisible(retracemem(x))$visible)
untracemem(x)
print(withVisible(retracemem(x)))

