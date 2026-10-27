### Name: curlGetHeaders
### Title: Retrieve Headers from URLs
### Aliases: curlGetHeaders

### ** Examples
## No test: 
##D ## needs Internet access, results vary
##D curlGetHeaders("http://bugs.r-project.org")   ## this redirects to https://
##D ## 2023-04: replaces slow and unreliable https://httpbin.org/status/404
##D curlGetHeaders("https://developer.R-project.org/inet-tests/not-found")
##D ## returns status
## End(No test)


