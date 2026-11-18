# Checks scripts/base-subset-messages.R (Rscript --vanilla, from the root).
source("scripts/base-subset-messages.R")
m <- function(a, b, part = "out") traced_differs("x.R", part, a, b, "saved.txt")
# the first differing line, on both sides
r <- m("a\nb\nc\n", "a\nB\nc\n")
stopifnot(grepl("its stdout differs at line 2.", r, fixed = TRUE),
          grepl("  untraced: b\n  traced:   B\n", r, fixed = TRUE),
          grepl("not a stale list", r, fixed = TRUE), grepl("Both transcripts: saved.txt", r, fixed = TRUE))
# a final newline alone (strsplit drops a trailing empty field)
r <- m("a", "a\n")
stopifnot(grepl("differs at line 2.", r, fixed = TRUE), grepl("(an empty line: a final newline)", r, fixed = TRUE),
          !grepl("NA", r, fixed = TRUE))
# one side a prefix of the other
r <- m("a\nb", "a\nb\nc", "err")
stopifnot(grepl("its stderr differs at line 3.", r, fixed = TRUE), grepl("untraced: (no line)", r, fixed = TRUE),
          grepl("traced:   c", r, fixed = TRUE))
r <- m("a\nb\n", "a\nb\nc\n")
stopifnot(grepl("untraced: (an empty line: a final newline)\n  traced:   c", r, fixed = TRUE))
# an empty line inside is not a final newline
r <- m("a\n\nb", "a\nX\nb")
stopifnot(grepl("untraced: (an empty line)\n", r, fixed = TRUE), !grepl("final newline", r, fixed = TRUE))
# the exit status
stopifnot(grepl("its exit status differs at line 1.", m("0", "1", "status"), fixed = TRUE))
cat("base-subset messages: ok\n")
