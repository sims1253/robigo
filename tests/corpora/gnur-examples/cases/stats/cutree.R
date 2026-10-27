RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: cutree
### Title: Cut a Tree into Groups of Data
### Aliases: cutree
### Keywords: multivariate cluster

### ** Examples

hc <- hclust(dist(USArrests))

cutree(hc, k = 1:5) #k = 1 is trivial
cutree(hc, h = 250)

## Compare the 2 and 4 grouping:
g24 <- cutree(hc, k = c(2,4))
table(grp2 = g24[,"2"], grp4 = g24[,"4"])



