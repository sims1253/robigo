# INTERIM(the writer streams): GNU R's R_Serialize writes to the connection
# as it walks, so an error in the walk leaves the bytes before it written:
# here the header, the list and 1:3 before the refhook's error at the
# environment. robigo-rt's writer works in memory (and may start its walk
# over at an effect point), so it writes nothing of a walk that fails. This
# case fails until the writer streams (docs/ledger/g3/2026-11-23-l5-saveload.md).
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-prefix-", Sys.getpid())
e <- new.env()
con <- file(f, "wb")
r <- tryCatch(serialize(list(1:3, e), con, refhook = function(x) stop("from the hook")),
              error = function(err) conditionMessage(err))
close(con)
print(r)
print(file.size(f))
invisible(file.remove(f))
