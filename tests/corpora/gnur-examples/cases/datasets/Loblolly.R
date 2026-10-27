RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Loblolly
### Title: Growth of Loblolly Pine Trees
### Aliases: Loblolly
### Keywords: datasets

### ** Examples

require(stats); require(graphics)
plot(height ~ age, data = Loblolly, subset = Seed == 329,
     xlab = "Tree age (yr)", las = 1,
     ylab = "Tree height (ft)",
     main = "Loblolly data and fitted curve (Seed 329 only)")
fm1 <- nls(height ~ SSasymp(age, Asym, R0, lrc),
           data = Loblolly, subset = Seed == 329)
age <- seq(0, 30, length.out = 101)
lines(age, predict(fm1, list(age = age)))



