library(grid)
### Name: resolveRasterSize
### Title: Utility function to resolve the size of a raster grob
### Aliases: resolveRasterSize
### Keywords: dplot

### ** Examples

# Square raster
rg <- rasterGrob(matrix(0))
# Fill the complete page (if page is square)
grid.newpage()
resolveRasterSize(rg)$height
grid.draw(rg)
# Forced to fit tall thin region
grid.newpage()
pushViewport(viewport(width=.1))
resolveRasterSize(rg)$height
grid.draw(rg)



