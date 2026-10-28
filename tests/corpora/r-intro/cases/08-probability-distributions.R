## An Introduction to R (GNU R 4.5.0), chapter: Probability distributions
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- R as a set of statistical tables
## 2-tailed p-value for t distribution
2*pt(-2.43, df = 13)
## upper 1% point for an F(2, 7) distribution
qf(0.01, 2, 7, lower.tail = FALSE)


## ---- Examining the distribution of a set of data
attach(faithful)
summary(eruptions)
fivenum(eruptions)
stem(eruptions)

hist(eruptions)
hist(eruptions, seq(1.6, 5.2, 0.2), prob=TRUE)
lines(density(eruptions, bw=0.1))
rug(eruptions) # show the actual data points

plot(ecdf(eruptions), do.points=FALSE, verticals=TRUE)

long <- eruptions[eruptions > 3]
plot(ecdf(long), do.points=FALSE, verticals=TRUE)
x <- seq(3, 5.4, 0.01)
lines(x, pnorm(x, mean=mean(long), sd=sqrt(var(long))), lty=3)

par(pty="s")       # arrange for a square figure region
qqnorm(long); qqline(long)

x <- rt(250, df = 5)
qqnorm(x); qqline(x)

qqplot(qt(ppoints(250), df = 5), x, xlab = "Q-Q plot for t dsn")
qqline(x)

shapiro.test(long)

ks.test(long, "pnorm", mean = mean(long), sd = sqrt(var(long)))
