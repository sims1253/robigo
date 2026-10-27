### Name: droplevels
### Title: Drop Unused Levels from Factors
### Aliases: droplevels droplevels.factor droplevels.data.frame
### Keywords: category NA

### ** Examples

aq <- transform(airquality, Month = factor(Month, labels = month.abb[5:9]))
aq <- subset(aq, Month != "Jul")
table(           aq $Month)
table(droplevels(aq)$Month)



