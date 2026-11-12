# R: serialize.c R_SerializeInfo, saveload.c do_loadFromConn2 (PRIMVAL 1).
# A version 3 header ends with the native encoding's length and name.
info <- function(x, ...) {
  con <- rawConnection(serialize(x, connection = NULL, ...))
  on.exit(close(con))
  str_info <- .Internal(serializeInfoFromConn(con))
  print(unlist(str_info))
}
info(42, version = 3)
info(42, version = 3, xdr = FALSE)
info(42, version = 3, ascii = TRUE)
info(42, version = 2)
info(42, version = 2, ascii = TRUE)

con <- rawConnection(raw(0), "r+")
a <- 42
save("a", file = con, version = 3)
seek(con, 0)
print(unlist(.Internal(loadInfoFromConn2(con))))
close(con)

con <- rawConnection(raw(0), "r+")
save("a", file = con, version = 3, ascii = TRUE)
seek(con, 0)
print(unlist(.Internal(loadInfoFromConn2(con))))
close(con)
