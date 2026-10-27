### Name: quartzFonts
### Title: Quartz Fonts Setup
### Aliases: quartzFont quartzFonts
### Keywords: device

### ** Examples
## No test: ##D 
##D if(.Platform$OS.type == "unix") { # includes macOS
##D 
##D   utils::str( quartzFonts() )  # a list(serif = .., sans = .., mono = ..)
##D   quartzFonts("mono")          # the list(mono = ..) sublist of quartzFonts()
##D 
## Not run: ##D 
##D ##D   ## for East Asian locales you can use something like
##D ##D   quartzFonts(sans = quartzFont(rep("AppleGothic", 4)),
##D ##D 	      serif = quartzFont(rep("AppleMyungjp", 4)))
##D ##D   ## since the default fonts may well not have the glyphs needed
## End(Not run)
##D 
##D }
## End(No test)


