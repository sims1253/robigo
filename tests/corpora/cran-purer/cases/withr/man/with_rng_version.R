## help-page examples 'with_rng_version', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_rng_version
### Title: RNG version
### Aliases: with_rng_version local_rng_version

### ** Examples

RNGkind()
with_rng_version("3.0.0", RNGkind())
with_rng_version("1.6.0", RNGkind())

with_rng_version("3.0.0",
  with_seed(42, sample(1:100, 3)))

with_rng_version("1.6.0",
  with_seed(42, sample(1:100, 3)))

RNGkind()

fun1 <- function() {
  local_rng_version("3.0.0")
  with_seed(42, sample(1:100, 3))
}

fun2 <- function() {
  local_rng_version("1.6.0")
  with_seed(42, sample(1:100, 3))
}

RNGkind()
fun1()
fun2()
RNGkind()



