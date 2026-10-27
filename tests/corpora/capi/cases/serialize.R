dyn.load("serialize.so")

con <- rawConnection(raw(0), "wb")
invisible(.Call("capi_serialize", list(1:3, c("a", "b"), 2.5), con))
bytes <- rawConnectionValue(con)
close(con)
print(length(bytes))
print(bytes[1:8])
