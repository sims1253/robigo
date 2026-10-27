RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: findCRANmirror
### Title: Find CRAN Mirror Preference
### Aliases: findCRANmirror
### Keywords: utilities

### ** Examples

## Don't show: 
old <- Sys.getenv("R_CRAN_WEB", NA_character_)
## End(Don't show)
c(findCRANmirror("src"), findCRANmirror("web"))

Sys.setenv(R_CRAN_WEB = "https://cloud.r-project.org")
c(findCRANmirror("src"), findCRANmirror("web"))
## Don't show: 
if(is.na(old)) Sys.setenv(R_CRAN_WEB = old) else Sys.unsetenv("R_CRAN_WEB")
## End(Don't show)



