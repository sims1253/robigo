RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: TukeyHSD
### Title: Compute Tukey Honest Significant Differences
### Aliases: TukeyHSD
### Keywords: models design

### ** Examples

require(graphics)

summary(fm1 <- aov(breaks ~ wool + tension, data = warpbreaks))
TukeyHSD(fm1, "tension", ordered = TRUE)
plot(TukeyHSD(fm1, "tension"))



