# Writes corrupt.rdb (run by GNU R 4.6.2 in this directory): four records of
# lazyLoadDBinsertValue's compressed form (type 1: a 4-byte big-endian
# length, then a zlib stream), for tests/rt/serialize/lazyload-corrupt.R:
#   1. good: the length and the stream of one serialized value;
#   2. bad data: the length, a zlib header, then bytes no stream starts with;
#   3. too few: a length 10 bytes short of what the stream inflates to;
#   4. too many: a length 100 bytes past it.
# Prints each record's key (offset, length).
p <- serialize(list(a = 1:3, b = "x"), NULL)
z <- memCompress(p, "gzip")
be <- function(n) writeBin(as.integer(n), raw(), size = 4L, endian = "big")
recs <- list(c(be(length(p)), z),
             c(be(length(p)), as.raw(c(0x78, 0x9c, rep(0xff, 20)))),
             c(be(length(p) - 10L), z),
             c(be(length(p) + 100L), z))
writeBin(do.call(c, recs), "corrupt.rdb")
print(rbind(offset = cumsum(c(0L, lengths(recs)))[seq_along(recs)], length = lengths(recs)))
