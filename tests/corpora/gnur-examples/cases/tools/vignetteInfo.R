RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: vignetteInfo
### Title: Basic Information about a Vignette
### Aliases: vignetteInfo
### Keywords: utilities

### ** Examples

  gridEx <- system.file("doc", "grid.Rnw", package = "grid")
  vi <- vignetteInfo(gridEx)
  str(vi)



