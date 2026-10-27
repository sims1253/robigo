### Name: png
### Title: BMP, JPEG, PNG and TIFF graphics devices
### Aliases: png jpeg tiff bmp
### Keywords: device

### ** Examples

## these examples will work only if the devices are available
## and cairo or an X11 display or a macOS display is available.

## copy current plot to a (large) PNG file
## Not run: dev.print(png, filename = "myplot.png", width = 1024, height = 768)
## No test: 
##D png("myplot.png", bg = "transparent")
##D plot(1:10)
##D rect(1, 5, 3, 7, col = "white")
##D dev.off()
##D 
##D ## will make myplot1.jpeg and myplot2.jpeg
##D jpeg("myplot%d.jpeg")
##D example(rect)
##D dev.off()
## End(No test)


