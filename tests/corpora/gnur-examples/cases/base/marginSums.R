RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: marginSums
### Title: Compute Table Margins
### Aliases: marginSums margin.table
### Keywords: array

### ** Examples

m <- matrix(1:4, 2)
marginSums(m, 1)  # = rowSums(m)
marginSums(m, 2)  # = colSums(m)

DF <- as.data.frame(UCBAdmissions)
tbl <- xtabs(Freq ~ Gender + Admit, DF)
tbl
marginSums(tbl, "Gender")  # a 1-dim "table"
rowSums(tbl)               # a numeric vector



