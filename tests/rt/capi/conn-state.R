# A connection's state is its struct Rconn (R_ext/Connections.h; the A5
# note's decision C): package C reads R's flags, mode, class and
# description through R_GetConnection, and what C writes there is what R
# sees (summary()'s "opened" after C clears isopen).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/conn_view.so"))
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-conn-state-", Sys.getpid())
con <- file(f, "w")
st <- .Call("rt_conn_state", con)
st[-7]
st[7] == f
summary(con)$opened
invisible(.Call("rt_conn_mark_closed", con))
summary(con)$opened
.Call("rt_conn_state", con)[1]
con2 <- file(f, "rb")
.Call("rt_conn_state", con2)[-7]
close(con2)
tryCatch(.Call("rt_conn_state", 99L), error = function(e) conditionMessage(e))
tryCatch(.Call("rt_conn_state", structure(99L, class = c("file", "connection"))), error = function(e) conditionMessage(e))
invisible(file.remove(f))
