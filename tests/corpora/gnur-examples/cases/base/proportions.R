### Name: proportions
### Title: Express Table Entries as Fraction of Marginal Table
### Aliases: proportions prop.table
### Keywords: array

### ** Examples

m <- matrix(1:4, 2)
m
proportions(m, 1)

DF <- as.data.frame(UCBAdmissions)
tbl <- xtabs(Freq ~ Gender + Admit, DF)
tbl
proportions(tbl, "Gender")



