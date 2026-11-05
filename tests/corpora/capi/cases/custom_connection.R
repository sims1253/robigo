dyn.load("custom_connection.so")

con <- .Call("cc_new", "one", "line one\nline two\nlast", 0L)
print(class(con))
s <- summary(con)
print(unlist(s[c("description", "class", "mode", "text", "opened", "can read", "can write")]))
print(isOpen(con))
print(.Call("cc_check", con))

# readLines opens a closed connection and closes it again (con->close,
# not destroy)
print(readLines(con))
print(isOpen(con))
open(con, "rb")
print(isOpen(con))
print(readBin(con, raw(), 4))
print(rawToChar(.Call("cc_read", con, 5L)))
print(isIncomplete(con))
print(readBin(con, raw(), 100))
print(isIncomplete(con))
close(con)  # close, then destroy

# an error from the open method; close() still destroys
two <- .Call("cc_new", "two", "x", 0L)
print(tryCatch(open(two, "w"), error = function(e) conditionMessage(e)))
print(isOpen(two))
close(two)

# not blocking: three bytes per read, incomplete lines pushed back
three <- .Call("cc_new", "three", "ab\ncd\nef", 3L)
open(three, "r", blocking = FALSE)
print(isIncomplete(three))
while (isIncomplete(three)) print(readLines(three))
close(three)

# a connection nothing refers to is closed and destroyed by gc(), with a
# warning if it was open
local({
    four <- .Call("cc_new", "four", "4", 0L)
    open(four)
})
invisible(NULL)
invisible(gc())
cat("after gc 1\n")
local({
    five <- .Call("cc_new", "five", "5", 0L)
})
invisible(NULL)
invisible(gc())
cat("after gc 2\n")

# with conn_id's address replaced (curl's handles), gc() leaves it
local({
    six <- .Call("cc_new", "six", "6", 0L)
    open(six)
    .Call("cc_replace_addr", six)
})
invisible(NULL)
invisible(gc())
cat("after gc 3\n")
# (getConnection(3) would read the dead conn_id, in GNU R too)
print(getAllConnections())
print(summary.connection(3L)$description)
close(structure(3L, class = c("memconn", "connection")))
