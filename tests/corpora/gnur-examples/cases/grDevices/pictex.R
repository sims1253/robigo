### Name: pictex
### Title: A PicTeX Graphics Driver
### Aliases: pictex
### Keywords: device

### ** Examples

require(graphics)
## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)

pictex()
plot(1:11, (-5:5)^2, type = "b", main = "Simple Example Plot")
dev.off()
##--------------------
## Not run: 
##D %% LaTeX Example
##D \documentclass{article}
##D \usepackage{pictex}
##D \usepackage{graphics} % for \rotatebox
##D \begin{document}
##D %...
##D \begin{figure}[h]
##D   \centerline{\input{Rplots.tex}}
##D   \caption{}
##D \end{figure}
##D %...
##D \end{document}
## End(Not run)
##--------------------
unlink("Rplots.tex")
## Don't show: 
setwd(oldwd)
## End(Don't show)



