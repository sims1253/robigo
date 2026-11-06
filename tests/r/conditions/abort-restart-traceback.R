# An error caught by an abort restart leaves .Traceback alone: errors.c's
# jump_to_top_ex invokes the restart before it records the traceback.
options(catch.script.errors = TRUE)
h <- function() stop("first")
h()
print(get(".Traceback", envir = baseenv()))
f <- function() stop("boom")
r <- withRestarts(f(), abort = function() "aborted")
print(r)
print(get(".Traceback", envir = baseenv()))
