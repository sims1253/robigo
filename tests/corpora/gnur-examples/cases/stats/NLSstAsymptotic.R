RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: NLSstAsymptotic
### Title: Fit the Asymptotic Regression Model
### Aliases: NLSstAsymptotic NLSstAsymptotic.sortedXyData
### Keywords: manip

### ** Examples

Lob.329 <- Loblolly[ Loblolly$Seed == "329", ]
print(NLSstAsymptotic(sortedXyData(expression(age),
                                   expression(height),
                                   Lob.329)), digits = 3)



