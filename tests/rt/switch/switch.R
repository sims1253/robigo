# switch: by name with fall-through and a default, by position, no match
switch("b", a = 1, b = 2, 3)
switch("z", a = 1, b = 2, 3)
switch(2, "x", "y", "z")
switch("a", a = , b = "ab", "d")
switch("q", a = 1)
f <- function(x) switch(x, a = return("early"), "late")
f("a")
f("b")
k <- 0
for (i in 1:4) switch(i, next, k <- k + 1, next, k <- k + 10)
k
