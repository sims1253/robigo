# system(intern = TRUE) pipes the child's stdout only (R_popen(cmd, "r"));
# its stderr passes through to R's own stderr. R CMD build ends with
# message("") there, so reg-packages' transcript has a blank line between
# the parent's warnings and R CMD INSTALL's output.
r <- system("sh -c 'echo out1; echo err1 >&2; echo out2'", intern = TRUE)
print(r)
print(attr(r, "status"))
# ignore.stderr appends "2>/dev/null" to the command: the shell discards it.
r <- system("sh -c 'echo out1; echo err2 >&2'", intern = TRUE, ignore.stderr = TRUE)
print(r)
# system2(stdout = TRUE) reaches the same internal with intern = TRUE.
r <- system2("sh", c("-c", shQuote("echo a; echo err3 >&2")), stdout = TRUE)
print(r)
# system2 with stdout = "" and stderr = "" inherits both streams.
print(system2("sh", c("-c", shQuote("echo c; echo err4 >&2"))))
# a non-zero exit status becomes the "status" attribute plus a warning.
r <- system("sh -c 'echo err5 >&2; exit 3'", intern = TRUE)
print(r)
print(attr(r, "status"))
