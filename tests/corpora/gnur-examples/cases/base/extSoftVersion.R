RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: extSoftVersion
### Title: Report Versions of Third-Party Software
### Aliases: extSoftVersion

### ** Examples

extSoftVersion()
## the PCRE version
sub(" .*", "", extSoftVersion()["PCRE"])



