# R_ExpandFileName's warning (an expansion longer than R_PATH_MAX) from a
# connection's open: do_url warns once, file_open again, before the open's
# own warning and error.
msg <- function(expr) withCallingHandlers(tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e)),
                                          warning = function(w) { cat("warning:", substr(conditionMessage(w), 1, 40), "\n"); invokeRestart("muffleWarning") })
long <- paste0("~root/", strrep("a", 4100))
print(msg(file(long, "r")))
con <- suppressWarnings(file(long))
print(msg(readLines(con)))
close(con)
print(msg(gzfile(long, "r")))
# R_ExpandFileName_readline: any name of R_PATH_MAX bytes or more is cut
# (the open's warning shows the name it tried)
print(msg(file(strrep("a", 5000), "r")))
