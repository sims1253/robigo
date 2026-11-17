# Names, environment keys and switch labels made by paste: looked up by
# their bytes, as in GNU R.
l <- list(1, 2); names(l) <- paste0("n", 1:2)
print(l$n2); print(l[["n1"]]); print(l[[paste0("n", 2)]])
e <- new.env()
for (i in 1:3) assign(paste0("v", i), i, envir = e)
print(sort(ls(e))); print(get(sprintf("v%d", 2), envir = e))
print(exists(paste0("v", 3), envir = e)); print(exists("v4", envir = e))
print(switch(paste0("c", 2), c1 = "one", c2 = "two"))
print(unique(list(paste0("u", 1), "u1", c("v", paste0("v", "")))))
print(.Internal(duplicated(list(paste0("u", 1), "u1"), FALSE, FALSE, NA)))
