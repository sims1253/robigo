## help-page examples 'with_par', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_par
### Title: Graphics parameters
### Aliases: with_par local_par

### ** Examples

old <- par("col" = "black")

# This will be in red
with_par(list(col = "red", pch = 19),
  plot(mtcars$hp, mtcars$wt)
)

# This will still be in black
plot(mtcars$hp, mtcars$wt)

par(old)



