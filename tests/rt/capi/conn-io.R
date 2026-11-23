# con->read and con->write from package C (R_ext/Connections.h) on a file
# connection: GNU R's file_read and file_write move between the read and
# write positions before fread/fwrite look at the request, so an empty read
# writes what is buffered; a read of a stream open only for writing (and a
# write of one open only for reading) is glibc's EBADF, in C's errno.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/conn_view.so"))
f <- tempfile()
con <- file(f, "w+b")
.Call("rt_conn_write", con, as.raw(65))
file.size(f)
.Call("rt_conn_read", con, 0L)
file.size(f)
.Call("rt_conn_write", con, raw(0))
.Call("rt_conn_read", con, 4L)
close(con)
con <- file(f, "wb")
.Call("rt_conn_read", con, 1L)
close(con)
con <- file(f, "rb")
.Call("rt_conn_write", con, as.raw(66))
.Call("rt_conn_read", con, 4L)
close(con)
file.size(f)
invisible(file.remove(f))
