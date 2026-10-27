### Name: octmode
### Title: Integer Numbers Displayed in Octal
### Aliases: as.octmode format.octmode print.octmode as.character.octmode
###   [.octmode !.octmode |.octmode &.octmode octmode
### Keywords: utilities print

### ** Examples

(on <- as.octmode(c(16, 32, 127:129))) # "020" "040" "177" "200" "201"
unclass(on[3:4]) # subsetting

## manipulate file modes
fmode <- as.octmode("170")
(fmode | "644") & "755"
## IGNORE_RDIFF_BEGIN
(umask <- Sys.umask()) # depends on platform
c(fmode, "666", "755") & !umask
## IGNORE_RDIFF_END

om <- as.octmode(1:12)
om # print()s via format()
stopifnot(nchar(format(om)) == 2)
om[1:7] # *no* leading zeroes!
stopifnot(format(om[1:7]) == as.character(1:7))
om2 <- as.octmode(c(1:10, 60:70))
om2 # prints via format() -> with 3 octals
stopifnot(nchar(format(om2)) == 3)
as.character(om2) # strings of length 1, 2, 3


## Integer arithmetic (remaining "octmode"):
om^2
om * 64
-om
(fac <- factorial(om)) # !1, !2, !3, !4 .. in hexadecimals
as.integer(fac) # indeed the same as  factorial(1:12)



