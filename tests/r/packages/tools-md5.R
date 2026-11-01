library(tools)

# RFC 1321's vectors, a multi-block input and binary bytes
print(tools::md5sum(bytes = charToRaw("")))
print(tools::md5sum(bytes = charToRaw("abc")))
print(tools::md5sum(bytes = charToRaw(strrep("a", 56))))
print(tools::md5sum(bytes = as.raw(1:300 %% 256)))
# files: one that exists, one that does not (NA)
f1 <- file.path(tempdir(), "robigo-md5-a.txt")
writeLines(c("hello", "world"), f1)
print(unname(tools::md5sum(f1)))
print(unname(tools::md5sum(c(f1, "/nonexistent/robigo-md5-nope"))))
print(tryCatch(tools::md5sum(bytes = 1:4), error = function(e) conditionMessage(e)))
unlink(f1)
