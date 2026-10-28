## help-page examples 'with_options', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_options
### Title: Options
### Aliases: with_options local_options

### ** Examples

# number of significant digits to print
getOption("digits")
# modify temporarily the number of significant digits to print
with_options(list(digits = 3), getOption("digits"))
with_options(list(digits = 3), print(pi))

# modify temporarily the character to be used as the decimal point
getOption("digits")
with_options(list(OutDec = ","), print(pi))

# modify temporarily multiple options
with_options(list(OutDec = ",", digits = 3), print(pi))

# modify, within the scope of the function, the number of
# significant digits to print
print_3_digits <- function(x) {
  # assign 3 to the option "digits" for the rest of this function
  # after the function exits, the option will return to its previous
  # value
  local_options(list(digits = 3))
  print(x)
}

print_3_digits(pi)  # returns 3.14
print(pi)           # returns 3.141593



