RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: pos.to.env
### Title: Convert Positions in the Search Path to Environments
### Aliases: pos.to.env
### Keywords: utilities

### ** Examples

pos.to.env(1) # R_GlobalEnv
# the next returns the base environment
pos.to.env(length(search()))



