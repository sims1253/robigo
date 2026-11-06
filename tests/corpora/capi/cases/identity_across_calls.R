dyn.load("identity.so")

address <- function(x) .Call("capi_id_addr", x)
same <- function(a, b) identical(address(a), address(b))

# every vector type, twice, and as an element of a list
vals <- list(lgl = c(TRUE, NA), int = c(4L, 5L), dbl = c(1.5, 2.5),
             cplx = c(1i, 2), chr = c("a", "b"), raw = as.raw(1:2),
             list = list(1, "a"), seq = 1L:3L)
for (nm in names(vals)) {
    x <- vals[[nm]]
    cat(nm, same(x, x), same(x, list(x)[[1]]), same(x, vals[[nm]]), "\n")
}
x <- 1L:3L
print(same(x, x))

# a C cbind of df1 against df[names(df1)], in both orders
df <- data.frame(a = 1:3, b = letters[1:3], c = c(1.5, 2.5, 3.5))
df1 <- df[c("a", "c")]
cb <- .Call("capi_id_cbind", df1, df["b"])
print(c(same(cb[[1]], df$a), same(cb[[2]], df$c), same(cb[[3]], df$b)))
cb <- .Call("capi_id_cbind", df["b"], df1)
print(c(same(cb[[1]], df$b), same(cb[[2]], df$a), same(cb[[3]], df[names(df1)][[2]])))

# data.table's .onLoad, verbatim but for printing instead of stop()
x = 1L:3L
y = list(x)
if (address(x) != address(y[[1L]])) cat("Unexpected base R behaviour: list(x) has copied x\n")

DF = data.frame(a=1:3, b=4:6)
add1 = address(DF$a)
add2 = address(DF$b)
names(DF) = c("A","B")
add3 = address(DF$A)
add4 = address(DF$B)
if (add1!=add3 || add2!=add4) cat("Unexpected base R behaviour: names<- has copied column contents\n")

DF = data.frame(a=1:3, b=4:6)
add1 = address(DF$a)
add2 = address(DF$b)
add3 = address(DF)
DF[2L, "b"] = 7L  # changed b but not a
add4 = address(DF$a)
add5 = address(DF$b)
add6 = address(DF)
if (add2==add5) cat("Unexpected base R behaviour: DF[i,j]<- didn't copy the column vector\n")
if (add1!=add4) cat("Unexpected base R behaviour: DF[i,j]<- has copied the other column vector\n")
if (add3==add6) cat("Unexpected base R behaviour: DF[i,j]<- has not copied address(DF)\n")
cat("done\n")
