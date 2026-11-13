show <- function(tag, x) cat(tag, ":", paste(x, collapse = " "), "\n")
prof <- function(tag, e) { p <- env.profile(e); cat(tag, ": size", p$size, "nchains", p$nchains, "buckets", paste(which(p$counts > 0) - 1L, collapse=","), "counts", paste(p$counts[p$counts > 0], collapse=","), "\n") }
# A: hashed, a few names
e <- new.env()
for (n in c("b", "a", "zz", "x1", "x2", "foo", "bar")) assign(n, 1, e)
show("A.ls", ls(e, sorted = FALSE)); prof("A.prof", e)
show("A.aslist", names(as.list(e)))
# B: rm and reinsertion
rm("a", envir = e); show("B.rm", ls(e, sorted = FALSE)); prof("B.prof", e)
assign("a", 2, e); show("B.re", ls(e, sorted = FALSE)); prof("B.prof2", e)
# C: resize
e <- new.env()
for (i in 1:100) assign(paste0("v", i), i, e)
show("C.ls", ls(e, sorted = FALSE)); prof("C.prof", e)
e <- new.env()
for (i in 1:24) assign(paste0("v", i), i, e); prof("C24", e)
assign("v25", 25, e); prof("C25", e)
assign("v26", 26, e); prof("C26", e); show("C26.ls", ls(e, sorted = FALSE))
# D: unhashed
e <- new.env(hash = FALSE)
for (n in c("b", "a", "c")) assign(n, 1, e)
show("D.ls", ls(e, sorted = FALSE)); rm("a", envir = e); assign("a", 1, e); show("D.re", ls(e, sorted = FALSE))
assign("b", 3, e); show("D.upd", ls(e, sorted = FALSE))
show("D.all", ls(e, all.names=TRUE, sorted=FALSE))
# E: dot names and all.names
e <- new.env(); for (n in c(".h", "a", ".b", "z")) assign(n, 1, e)
show("E.ls", ls(e, sorted = FALSE)); show("E.all", ls(e, all.names = TRUE, sorted = FALSE)); show("E.sorted", ls(e, all.names=TRUE))
# F: non-ASCII names (signed char in R_Newhashpjw)
e <- new.env()
for (n in c("é", "ü", "aéb", "中文", "x")) assign(n, 1, e)
show("F.ls", ls(e, sorted = FALSE)); prof("F.prof", e)
# G: sizes
e <- new.env(size = 5L); for (n in letters[1:8]) assign(n, 1, e); prof("G.prof5", e); show("G.ls", ls(e, sorted=FALSE))
e <- new.env(size = NA); prof("G.na", e)
e <- new.env(size = 0L); prof("G.0", e)
# H: global env in a vanilla session
show("H.global", ls(globalenv(), all.names = TRUE, sorted = FALSE))
x <- 1; b <- 2; a <- 3
show("H.global2", ls(globalenv(), sorted = FALSE)); prof("H.prof", globalenv())
# I: collisions: names with same bucket in size 29
e <- new.env()
nm <- c("a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w")
for (n in nm) assign(n, 1, e); prof("I.prof", e); show("I.ls", ls(e, sorted=FALSE))
for (n in c("aa","ab","ac","ad","ae")) assign(n, 1, e); prof("I.prof2", e); show("I.ls2", ls(e, sorted=FALSE))
# J: mget order and list2env
e <- list2env(list(q = 1, w = 2, e = 3, r = 4)); show("J.ls", ls(e, sorted=FALSE)); prof("J.prof", e)
e <- list2env(list(q = 1, w = 2, e = 3, r = 4), hash = FALSE); show("J.unh", ls(e, sorted=FALSE))
