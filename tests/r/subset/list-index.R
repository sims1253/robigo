# `[`, `[[` and `$` on lists: names, partial matching, recursive indexing.
l <- list(alpha = 1, beta = "two", gamma = list(delta = 4, eps = 5:7))
l["alpha"]
l[c("beta", "alpha")]
l[2]
l[-1]
l[c(TRUE, FALSE, TRUE)]
l[c("alpha", "zzz")]
l[5]
l[[1]]
l[["beta"]]
l[["gamma"]][["eps"]]
l[["gamma"]]$eps[2]
l$gamma$delta
l[[c(3, 2)]]
l[[c("gamma", "eps")]]
l[[c(3, 2, 3)]]
l[["zzz"]]
l$zzz
l[[NA]]
l[[NA_character_]]
# partial matching: `$` partially matches, `[[` only with exact = FALSE
l$al
l$g$de
l[["al"]]
l[["al", exact = FALSE]]
l[["al", exact = TRUE]]
amb <- list(abc = 1, abd = 2)
amb$ab
amb[["ab", exact = FALSE]]
amb$abc
# NULL elements and unnamed lists
u <- list(1, NULL, "c")
length(u)
u[2]
u[[2]]
u[-2]
u$a
names(u)
# lists with partly empty names
pn <- list(a = 1, 2, b = 3)
pn[c("a", "b")]
pn[[2]]
pn$b
names(pn[2:3])
# lists keep their names and drop other attributes
al <- list(x = 1, y = 2)
attr(al, "meta") <- "m"
attributes(al[2])
class(al) <- "rec"
inherits(al[1], "rec")
inherits(al[[1]], "rec")
# drop = is accepted and ignored for vectors
l[2, drop = FALSE]
l[[3, drop = TRUE]]$delta
