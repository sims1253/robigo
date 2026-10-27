RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: URLencode
### Title: Encode or Decode (partial) URLs
### Aliases: URLencode URLdecode
### Keywords: utilities

### ** Examples

(y <- URLencode("a url with spaces and / and @"))
URLdecode(y)
(y <- URLencode("a url with spaces and / and @", reserved = TRUE))
URLdecode(y)

URLdecode(z <- "ab%20cd")
c(URLencode(z), URLencode(z, repeated = TRUE)) # first is usually wanted

## both functions support character vectors of length > 1
y <- URLdecode(URLencode(c("url with space", "another one")))



