# needs: bayesgrove
# The user's own package (github.com/sims1253/bayesgrove): graph-based
# Bayesian workflow scaffolding. Exercises project init and status
# without a fitting backend.
suppressPackageStartupMessages(library(bayesgrove))
tmp <- tempfile("bg-smoke-")
dir.create(tmp)
h <- bg_init(path = tmp)
print(is.list(bg_status(h)))
unlink(tmp, recursive = TRUE)
