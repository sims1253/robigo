### Name: srcfile
### Title: References to Source Files and Code
### Aliases: srcfile srcfilecopy getSrcLines srcref srcfile-class
###   srcfilecopy-class srcref-class print.srcfile summary.srcfile
###   open.srcfile open.srcfilecopy close.srcfile print.srcref
###   summary.srcref as.character.srcref .isOpen srcfilealias-class
###   srcfilealias open.srcfilealias close.srcfilealias
### Keywords: debugging utilities

### ** Examples

src <- srcfile(system.file("DESCRIPTION", package = "base"))
## IGNORE_RDIFF_BEGIN
summary(src)
## IGNORE_RDIFF_END
getSrcLines(src, 1, 4)
ref <- srcref(src, c(1, 1, 2, 1000))
ref
print(ref, useSource = FALSE)



