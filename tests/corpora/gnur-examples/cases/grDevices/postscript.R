RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: postscript
### Title: PostScript Graphics
### Aliases: postscript .ps.prolog
### Keywords: device

### ** Examples

require(graphics)
## Not run: 
##D # open the file "foo.ps" for graphics output
##D postscript("foo.ps")
##D # produce the desired graph(s)
##D dev.off()              # turn off the postscript device
##D 
##D ## On Unix-alikes only:
##D postscript("|lp -dlw")
##D # produce the desired graph(s)
##D dev.off()              # plot will appear on printer
##D 
##D ## On Windows:
##D options(printcmd = 'redpr -P"\\printhost\lw"')
##D postscript(file = tempfile("Rps."), print.it = TRUE)
##D # produce the desired graph(s)
##D dev.off()              # send plot file to the printer
##D ## alternative using GSView 4.x :
##D options(printcmd = '/GhostGum/gsview/gsprint -query')
##D 
##D 
##D # for URW PostScript devices
##D postscript("foo.ps", family = "NimbusSan")
##D 
##D ## for inclusion in Computer Modern TeX documents, perhaps
##D postscript("cm_test.eps", width = 4.0, height = 3.0,
##D            horizontal = FALSE, onefile = FALSE, paper = "special",
##D            family = "ComputerModern", encoding = "TeXtext.enc")
##D ## The resultant postscript file can be used by dvips -Ppfb -j0.
##D 
##D ## To test out encodings, you can use
##D TestChars <- function(encoding = "ISOLatin1", family = "URWHelvetica")
##D {
##D     postscript(encoding = encoding, family = family)
##D     par(pty = "s")
##D     plot(c(-1,16), c(-1,16), type = "n", xlab = "", ylab = "",
##D          xaxs = "i", yaxs = "i")
##D     title(paste("Centred chars in encoding", encoding))
##D     grid(17, 17, lty = 1)
##D     for(i in c(32:255)) {
##D         x <- i %% 16
##D         y <- i %/% 16
##D         points(x, y, pch = i)
##D     }
##D     dev.off()
##D }
##D ## there will be many warnings.  We use URW to get a complete enough
##D ## set of font metrics.
##D TestChars()
##D TestChars("ISOLatin2")
##D TestChars("WinAnsi")
## End(Not run)
## Don't show: 
xx <- seq(0, 7, length = 701)
yy <- sin(xx)/xx; yy[1] <- 1
plot(xx,yy)                     # produce the desired graph(s)
## End(Don't show)



