# `local` is one of the language functions GNU R's compiler inlines, so the
# interpreter takes base's `local` in the frame of a closure GNU R would
# have compiled (one with a loop). Base's binding is lazily loaded (a
# forced promise): the function is its value.
g <- function() { for (i in 1:2) local(x <- i); 3 }
print(g())
h <- function() { while (TRUE) { local(1); break }; local(4) }
print(h())
