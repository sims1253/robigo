## An Introduction to R (GNU R 4.5.0), chapter: Statistical models in R
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Defining statistical models; formulae
options(contrasts = c("contr.treatment", "contr.poly"))

options(contrasts = c("contr.helmert", "contr.poly"))


## ---- Analysis of variance and model comparison
response ~ mean.formula + Error(strata.formula)


## ---- Generalized linear models
kalythos <- data.frame(x = c(20,35,45,55,70), n = rep(50,5),
                         y = c(6,17,26,37,44))

kalythos$Ymat <- cbind(kalythos$y, kalythos$n - kalythos$y)

fmp <- glm(Ymat ~ x, family = binomial(link=probit), data = kalythos)
fml <- glm(Ymat ~ x, family = binomial, data = kalythos)

summary(fmp)
summary(fml)

ld50 <- function(b) -b[1]/b[2]
ldp <- ld50(coef(fmp)); ldl <- ld50(coef(fml)); c(ldp, ldl)


## ---- Nonlinear least squares and maximum likelihood models
x <- c(0.02, 0.02, 0.06, 0.06, 0.11, 0.11, 0.22, 0.22, 0.56, 0.56,
         1.10, 1.10)
y <- c(76, 47, 97, 107, 123, 139, 159, 152, 191, 201, 207, 200)

fn <- function(p) sum((y - (p[1] * x)/(p[2] + x))^2)

plot(x, y)
xfit <- seq(.02, 1.1, .05)
yfit <- 200 * xfit/(0.1 + xfit)
lines(spline(xfit, yfit))

out <- nlm(fn, p = c(200, 0.1), hessian = TRUE)

sqrt(diag(2*out$minimum/(length(y) - 2) * solve(out$hessian)))

plot(x, y)
xfit <- seq(.02, 1.1, .05)
yfit <- 212.68384222 * xfit/(0.06412146 + xfit)
lines(spline(xfit, yfit))

df <- data.frame(x=x, y=y)
fit <- nls(y ~ SSmicmen(x, Vm, K), df)
fit
summary(fit)

x <- c(1.6907, 1.7242, 1.7552, 1.7842, 1.8113,
         1.8369, 1.8610, 1.8839)
y <- c( 6, 13, 18, 28, 52, 53, 61, 60)
n <- c(59, 60, 62, 56, 63, 59, 62, 60)

out <- nlm(fn, p = c(-50,20), hessian = TRUE)

sqrt(diag(solve(out$hessian)))
