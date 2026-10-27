RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: Rdutils
### Title: Rd Utilities
### Aliases: Rd_db
### Keywords: utilities documentation

### ** Examples
## No test: 
##D ## Build the Rd db for the (installed) base package.
##D db <- Rd_db("base")
##D 
##D ## Keyword metadata per Rd object.
##D keywords <- lapply(db, tools:::.Rd_get_metadata, "keyword")
##D ## Tabulate the keyword entries.
##D kw_table <- sort(table(unlist(keywords)))
##D ## The 5 most frequent ones:
##D rev(kw_table)[1 : 5]
##D ## The "most informative" ones:
##D kw_table[kw_table == 1]
##D 
##D ## Concept metadata per Rd file.
##D concepts <- lapply(db, tools:::.Rd_get_metadata, "concept")
##D ## How many files already have \concept metadata?
##D sum(sapply(concepts, length) > 0)
##D ## How many concept entries altogether?
##D length(unlist(concepts))
## End(No test)


