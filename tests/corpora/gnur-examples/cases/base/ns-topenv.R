RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ns-topenv
### Title: Top Level Environment
### Aliases: topenv
### Keywords: programming

### ** Examples

topenv(.GlobalEnv)
topenv(new.env()) # also global env
topenv(environment(ls))# namespace:base
topenv(environment(lm))# namespace:stats
## Don't show: 
stopifnot(identical(.GlobalEnv,        topenv(new.env())),
          identical(.GlobalEnv,        topenv(.GlobalEnv)),
          identical(baseenv(),         topenv(baseenv())),
          identical(.BaseNamespaceEnv, topenv(.BaseNamespaceEnv)),
          identical(topenv(environment(ls)), asNamespace("base")),
          identical(topenv(environment(lm)), asNamespace("stats")))
## End(Don't show)



