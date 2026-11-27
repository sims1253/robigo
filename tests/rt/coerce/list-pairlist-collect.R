# List and pairlist coercions on Cx::fill_list, Cx::fill_pairlist, and
# Cx::fill_strings: long enough to collect repeatedly under gctorture and
# young:16 while keeping torture runtime small.
nms <- paste0("n", seq_len(400))

# atomic -> list (names kept, other attributes dropped)
x <- structure(seq_len(400), names = nms, extra = "drop")
l <- as.list(x)
length(l); typeof(l); l[[1]]; l[[400]]; names(l)[c(1, 400)]; attr(l, "extra")

# atomic -> expression (no names)
e <- as.expression(x)
length(e); typeof(e); e[[1]]; e[[400]]; attributes(e)

# atomic -> pairlist (names as tags, "" and NA skipped)
v <- seq_len(400) * 1.5
vn <- nms
vn[2] <- ""
vn[3] <- NA_character_
names(v) <- vn
p1 <- as.pairlist(v)
length(p1); typeof(p1); p1[[1]]; p1[[2]]; p1[[3]]; p1[[400]]
names(p1)[c(1, 2, 3, 4, 400)]

# list -> pairlist ("" skipped, NA kept as `NA` tag per VectorToPairList)
lv <- as.list(seq_len(400))
names(lv) <- vn
p2 <- as.pairlist(lv)
length(p2); typeof(p2); p2[[1]]; p2[[400]]
names(p2)[c(1, 2, 3, 4, 400)]

# list / pairlist / call -> character via fill_strings
sc <- as.character(lv)
length(sc); sc[c(1, 400)]; attributes(sc)
sp <- as.character(p2)
length(sp); sp[c(1, 400)]; attributes(sp)
cl <- as.call(c(list(quote(f)), as.list(seq_len(100))))
scl <- as.character(cl)
length(scl); scl[c(1, 2, 101)]

# empty inputs
as.list(integer(0)); as.expression(integer(0)); as.pairlist(integer(0)); as.pairlist(list())

# mapply MoreArgs through vector_to_pair_list_fill
ma <- as.list(1:20)
names(ma) <- paste0("a", 1:20)
mapply(function(...) length(list(...)), 1:2, MoreArgs = ma)
