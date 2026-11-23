for (i in 1:3) print(i)
for (s in c("a", "b")) cat(s, "\n")
i <- 0
while (i < 3) {
  i <- i + 1
  cat("i =", i, "\n")
}
repeat {
  i <- i - 1
  if (i == 0) break
}
i
for (k in 1:10) {
  if (k %% 2 == 0) next
  if (k > 7) break
  cat(k, "")
}
cat("\n")
x <- if (TRUE) "yes" else "no"
x
y <- if (FALSE) 1
y
f <- function(n) {
  for (i in seq_len(n)) if (i == 3) return(i * 100)
  -1
}
f(5)
f(2)
switch("b", a = "A", b = "B", "other")
switch("z", a = "A", b = "B", "other")
switch(2, "one", "two", "three")
switch("a", a = , b = "fallthrough")
is.null(switch("q", a = 1))
for (x in list(1, "a", TRUE)) print(class(x))
res <- vapply(1:3, function(i) i * 2, numeric(1))
res
