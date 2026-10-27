RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: as.raster
### Title: Create a Raster Object
### Aliases: is.raster as.raster as.raster.logical as.raster.numeric
###   as.raster.raw as.raster.character as.raster.matrix as.raster.array
### Keywords: dplot

### ** Examples

# A red gradient
as.raster(matrix(hcl(0, 80, seq(50, 80, 10)),
                 nrow = 4, ncol = 5))

# Vectors are 1-column matrices ...
#   character vectors are color names ...
as.raster(hcl(0, 80, seq(50, 80, 10)))
#   numeric vectors are greyscale ...
as.raster(1:5, max = 5)
#   logical vectors are black and white ...
as.raster(1:10 %% 2 == 0)

# ... unless nrow/ncol are supplied ...
as.raster(1:10 %% 2 == 0, nrow = 1)

# Matrix can also be logical or numeric (or raw) ...
as.raster(matrix(c(TRUE, FALSE), nrow = 3, ncol = 2))
as.raster(matrix(1:3/4, nrow = 3, ncol = 4))

# An array can be 3-plane numeric (R, G, B planes) ...
as.raster(array(c(0:1, rep(0.5, 4)), c(2, 1, 3)))

# ... or 4-plane numeric (R, G, B, A planes)
as.raster(array(c(0:1, rep(0.5, 6)), c(2, 1, 4)))

# subsetting
r <- as.raster(matrix(colors()[1:100], ncol = 10))
r[, 2]
r[2:4, 2:5]

# assigning to subset
r[2:4, 2:5] <- "white"

# comparison
r == "white"

## Don't show: 
stopifnot(r[] == r,
          identical(r[3:5], colors()[3:5]))
r[2:4] <- "black"
stopifnot(identical(r[1:4, 1], as.raster(c("white", rep("black", 3)))))
## End(Don't show)



