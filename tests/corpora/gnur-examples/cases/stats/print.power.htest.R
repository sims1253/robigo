RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: print.power.htest
### Title: Print Methods for Hypothesis Tests and Power Calculation Objects
### Aliases: print.htest print.power.htest
### Keywords: htest

### ** Examples

(ptt <- power.t.test(n = 20, delta = 1))
print(ptt, digits =  4) # using less digits than default
print(ptt, digits = 12) # using more  "       "     "



