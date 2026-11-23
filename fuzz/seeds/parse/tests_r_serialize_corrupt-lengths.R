# A corrupt file is an error to catch, not a reason to end the session.
caught <- function(expr) {
    cat(tryCatch({ expr; "read" }, error = function(e) "error"), "\n")
}
patch <- function(obj, offset, bytes) {
    r <- serialize(obj, NULL, version = 2)
    r[offset + seq_along(bytes) - 1L] <- bytes
    r
}
# XDR header: 14 bytes, then flags, then the length.
long <- as.raw(c(0xff, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff))
for (x in list(c(1.5, 2.5), c(1L, 2L), c("a", "b"), list(1, 2), c(1i, 2i), as.raw(1:2))) {
    caught(unserialize(patch(x, 19L, long)))
}
# Truncated in the middle of an element.
r <- serialize(c(1.5, 2.5), NULL, version = 2)
caught(unserialize(r[seq_len(length(r) - 3L)]))
cat("survived\n")
