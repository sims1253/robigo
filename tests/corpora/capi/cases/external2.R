dyn.load("external2.so")  # runs R_init_external2

# .External2 keeps the visibility of the routine's last evaluation.
print(unlist(withVisible(.External2("ext2_eval", quote(a <- 1)))))
print(unlist(withVisible(.External2("ext2_eval", quote(invisible(2))))))
print(unlist(withVisible(.External2("ext2_eval", quote(3)))))
print(unlist(withVisible(.External2("ext2_eval", quote(b <- 4), quote(b + 1)))))
print(unlist(withVisible(.External2("ext2_eval", quote(5), quote(v <- 6)))))
# no evaluation: the visibility the arguments' evaluation left
print(unlist(withVisible(.External2("ext2_eval"))))

# through a closure, as magrittr's pipes call it; top level prints only
# visible results
f <- function(...) .External2("ext2_eval", ...)
f(quote(d <<- 7))
f(quote(8))
f(quote(invisible(9)))
print(c(a, b, v, d))

# .External is always visible.
print(unlist(withVisible(.External("ext1_eval", quote(e <- 10)))))
g <- function(...) .External("ext1_eval", ...)
g(quote(h <- 11))
print(c(e, h))
