RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: dotchart
### Title: Cleveland's Dot Plots
### Aliases: dotchart
### Keywords: hplot

### ** Examples

dotchart(VADeaths, main = "Death Rates in Virginia - 1940")

op <- par(xaxs = "i")  # 0 -- 100%
dotchart(t(VADeaths), xlim = c(0,100), bg = "skyblue",
         main = "Death Rates in Virginia - 1940", xlab = "rate [ % ]",
         ylab = "Grouping:  Age  x   Urbanity . Gender")
par(op)



