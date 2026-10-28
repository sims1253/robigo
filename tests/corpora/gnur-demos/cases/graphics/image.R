## GNU R 4.5.0 demo(image) -- src/library/graphics/demo/image.R.
## Part of R, GPL-2 or later. Run as demo("image", package="graphics",
## ask = FALSE, echo = TRUE) runs it: the demo() banner, then the file
## itself with echo (the corpus's echo mode supplies the prompts).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)
cat("\n\n\tdemo(image)\n\t---- ~~~~~\n", sep = "")
#  Copyright (C) 1997-2009 The R Core Team

require(datasets)
require(grDevices); require(graphics)

x <- 10*(1:nrow(volcano)); x.at <- seq(100, 800, by=100)
y <- 10*(1:ncol(volcano)); y.at <- seq(100, 600, by=100)

					# Using Terrain Colors

image(x, y, volcano, col=terrain.colors(100),axes=FALSE)
contour(x, y, volcano, levels=seq(90, 200, by=5), add=TRUE, col="brown")
axis(1, at=x.at)
axis(2, at=y.at)
box()
title(main="Maunga Whau Volcano", sub = "col=terrain.colors(100)", font.main=4)

					# Using Heat Colors

image(x, y, volcano, col=heat.colors(100), axes=FALSE)
contour(x, y, volcano, levels=seq(90, 200, by=5), add=TRUE, col="brown")
axis(1, at=x.at)
axis(2, at=y.at)
box()
title(main="Maunga Whau Volcano", sub = "col=heat.colors(100)", font.main=4)

					# Using Gray Scale

image(x, y, volcano, col=gray(100:200/200), axes=FALSE)
contour(x, y, volcano, levels=seq(90, 200, by=5), add=TRUE, col="black")
axis(1, at=x.at)
axis(2, at=y.at)
box()
title(main="Maunga Whau Volcano \n col=gray(100:200/200)", font.main=4)

## Filled Contours are even nicer sometimes :
example(filled.contour)
