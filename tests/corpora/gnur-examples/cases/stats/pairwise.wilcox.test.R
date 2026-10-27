RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: pairwise.wilcox.test
### Title: Pairwise Wilcoxon Rank Sum Tests
### Aliases: pairwise.wilcox.test
### Keywords: htest

### ** Examples

attach(airquality)
Month <- factor(Month, labels = month.abb[5:9])
## These give warnings because of ties :
pairwise.wilcox.test(Ozone, Month)
pairwise.wilcox.test(Ozone, Month, p.adjust.method = "bonf")
detach()



