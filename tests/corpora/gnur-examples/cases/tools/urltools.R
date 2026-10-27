RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: urltools
### Title: Check Package URLs
### Aliases: check_package_urls parse_URI_reference
### Keywords: utilities

### ** Examples

## Examples from RFC 3986.
parse_URI_reference(c("foo://example.com:8042/over/there?name=ferret#nose",
                      "urn:example:animal:ferret:nose",
                      "mailto:John.Doe@example.com",
                      "tel:+1-816-555-1212"))



