### Name: X11Fonts
### Title: X11 Fonts
### Aliases: X11Font X11Fonts
### Keywords: device

### ** Examples
## IGNORE_RDIFF_BEGIN
if(capabilities("X11")) withAutoprint({
X11Fonts()
X11Fonts("mono")
utopia <- X11Font("-*-utopia-*-*-*-*-*-*-*-*-*-*-*-*")
X11Fonts(utopia = utopia)
})
## IGNORE_RDIFF_END


