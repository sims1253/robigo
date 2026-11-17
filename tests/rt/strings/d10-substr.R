# substr's results, then compared and hashed as GNU R compares cached
# CHARSXPs.
s <- .Internal(substr(c("hello", "help", "he"), 1L, 3L))
print(s)
print(s == "hel")
print(unique(s))
print(match("hel", s))
print(identical(s[1], s[2]))
w <- .Internal(substr(c("whole", "whole"), 1L, 5L))
print(w); print(unique(w))
