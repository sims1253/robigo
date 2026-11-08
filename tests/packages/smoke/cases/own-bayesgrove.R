# needs: bayesgrove
# The user's own package (github.com/sims1253/bayesgrove): the Bayesian
# workflow as a graph. Its README example fits Stan models; this keeps
# what needs no fitting backend: a project, its status, and the API
# boundary table the README points to.
suppressPackageStartupMessages(library(bayesgrove))
tmp <- tempfile("bg-smoke-")
dir.create(tmp)
h <- bg_init(path = tmp)
print(names(bg_status(h)))
print(bg_api_boundary())
unlink(tmp, recursive = TRUE)
