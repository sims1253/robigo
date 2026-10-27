RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: windowsFonts
### Title: Windows Fonts
### Aliases: windowsFont windowsFonts
### Keywords: device

### ** Examples
## No test: ##D 
##D if(.Platform$OS.type == "windows") withAutoprint({
##D   windowsFonts()
##D   windowsFonts("mono")
##D })
##D 
## Not run: 
##D ##D ## set up for Japanese: needs the fonts installed
##D ##D windows()  # make sure we have the right device type (available on Windows only)
##D ##D Sys.setlocale("LC_ALL", "ja")
##D ##D windowsFonts(JP1 = windowsFont("MS Mincho"),
##D ##D              JP2 = windowsFont("MS Gothic"),
##D ##D              JP3 = windowsFont("Arial Unicode MS"))
##D ##D plot(1:10)
##D ##D text(5, 2, "\u{4E10}\u{4E00}\u{4E01}", family = "JP1")
##D ##D text(7, 2, "\u{4E10}\u{4E00}\u{4E01}", family = "JP1", font = 2)
##D ##D text(5, 1.5, "\u{4E10}\u{4E00}\u{4E01}", family = "JP2")
##D ##D text(9, 2, "\u{5100}", family = "JP3")
## End(Not run)
##D 
## End(No test)


