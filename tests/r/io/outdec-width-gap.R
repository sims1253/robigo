## OutDec must be one character wide. options() warns when it is not
## (options.c), and EncodeReal0 warns again when it actually substitutes the
## mark ("the decimal mark is more than one character wide; this will become
## an error", printutils.c). Gap: robigo implements the options() warning but
## not EncodeReal0's, and it does not spell out a multi-character mark.
op <- options(OutDec = ".1.")
print(format(pi))
print(as.character(pi))
cat(paste0(pi), "\n")
options(OutDec = "")
print(format(pi))
options(op)
