RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getGraphicsEvent
### Title: Wait for a mouse or keyboard event from a graphics window
### Aliases: getGraphicsEvent setGraphicsEventHandlers getGraphicsEventEnv
###   setGraphicsEventEnv
### Keywords: iplot

### ** Examples

# This currently only works on the Windows, X11(type = "Xlib"), and
# X11(type = "cairo") screen devices...
## Not run: 
##D savepar <- par(ask = FALSE)
##D dragplot <- function(..., xlim = NULL, ylim = NULL, xaxs = "r", yaxs = "r") {
##D     plot(..., xlim = xlim, ylim = ylim, xaxs = xaxs, yaxs = yaxs)
##D     startx <- NULL
##D     starty <- NULL
##D     prevx <- NULL
##D     prevy <- NULL
##D     usr <- NULL
##D 
##D     devset <- function()
##D         if (dev.cur() != eventEnv$which) dev.set(eventEnv$which)
##D 
##D     dragmousedown <- function(buttons, x, y) {
##D         startx <<- x
##D         starty <<- y
##D         prevx <<- 0
##D         prevy <<- 0
##D         devset()
##D         usr <<- par("usr")
##D         eventEnv$onMouseMove <- dragmousemove
##D         NULL
##D     }
##D 
##D     dragmousemove <- function(buttons, x, y) {
##D         devset()
##D         deltax <- diff(grconvertX(c(startx, x), "ndc", "user"))
##D         deltay <- diff(grconvertY(c(starty, y), "ndc", "user"))
##D 	if (abs(deltax-prevx) + abs(deltay-prevy) > 0) {
##D 	    plot(..., xlim = usr[1:2]-deltax, xaxs = "i",
##D 		      ylim = usr[3:4]-deltay, yaxs = "i")
##D 	    prevx <<- deltax
##D 	    prevy <<- deltay
##D 	}
##D         NULL
##D     }
##D 
##D     mouseup <- function(buttons, x, y) {
##D     	eventEnv$onMouseMove <- NULL
##D     }	
##D 
##D     keydown <- function(key) {
##D         if (key == "q") return(invisible(1))
##D         eventEnv$onMouseMove <- NULL
##D         NULL
##D     }
##D 
##D     setGraphicsEventHandlers(prompt = "Click and drag, hit q to quit",
##D                      onMouseDown = dragmousedown,
##D                      onMouseUp = mouseup,
##D                      onKeybd = keydown)
##D     eventEnv <- getGraphicsEventEnv()
##D }
##D 
##D dragplot(rnorm(1000), rnorm(1000))
##D getGraphicsEvent()
##D par(savepar)
## End(Not run)



