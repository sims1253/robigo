# A connection's conn_id is an external pointer whose address is the
# connection's id (connections.c con->id), one more for each new
# connection; a serialized one comes back as a NULL pointer. parallelly's
# connectionId() reads the address back from the printed form, and future
# treats 0x0 / (nil) as "serialized": resolved() of a multisession future
# was never TRUE, so future_lapply() hung.
addr <- function(p) strtoi(sub("<pointer: 0x(.*)>", "\\1", p), 16L)
a <- file(tempfile(), "w")
b <- file(tempfile(), "w")
ia <- addr(deparse(attr(a, "conn_id")))
ib <- addr(format(attr(b, "conn_id")))
print(c(positive = ia > 0, consecutive = ib == ia + 1L))
close(a)
close(b)
p <- unserialize(serialize(attr(file(tempfile(), "w"), "conn_id"), NULL))
print(p)
print(deparse(p))
print(format(p))
