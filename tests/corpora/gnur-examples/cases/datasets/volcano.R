RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: volcano
### Title: Topographic Information on Auckland's Maunga Whau Volcano
### Aliases: volcano
### Keywords: datasets

### ** Examples

require(grDevices); require(graphics)
filled.contour(volcano, color.palette = terrain.colors, asp = 1)
title(main = "volcano data: filled contour map")



