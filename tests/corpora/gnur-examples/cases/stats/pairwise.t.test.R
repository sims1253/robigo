RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: pairwise.t.test
### Title: Pairwise t tests
### Aliases: pairwise.t.test
### Keywords: htest

### ** Examples

attach(airquality)
Month <- factor(Month, labels = month.abb[5:9])
pairwise.t.test(Ozone, Month)
pairwise.t.test(Ozone, Month, p.adjust.method = "bonf")
pairwise.t.test(Ozone, Month, pool.sd = FALSE)
detach()



