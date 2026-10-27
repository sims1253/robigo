RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.setenv
### Title: Set or Unset Environment Variables
### Aliases: Sys.setenv Sys.unsetenv
### Keywords: environment utilities

### ** Examples

print(Sys.setenv(R_TEST = "testit", "A+C" = 123))  # `A+C` could also be used
Sys.getenv("R_TEST")
Sys.unsetenv("R_TEST") # on Unix-alike may warn and not succeed
Sys.getenv("R_TEST", unset = NA)



