RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ARMAtoMA
### Title: Convert ARMA Process to Infinite MA Process
### Aliases: ARMAtoMA
### Keywords: ts

### ** Examples

ARMAtoMA(c(1.0, -0.25), 1.0, 10)
## Example from Brockwell & Davis (1991, p.92)
## answer (1 + 3*n)*2^(-n)
n <- 1:10; (1 + 3*n)*2^(-n)



