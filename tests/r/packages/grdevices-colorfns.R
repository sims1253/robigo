library(grDevices)
# gray(), rgb(), hsv(), hcl() and rgb2hsv(): recycling, scaling and
# every error path of grDevices/src/colors.c's converters. The pillar
# and scales packages call gray()/hcl() while loading, so these are on
# the path of library(tibble) and library(ggplot2).

# gray: levels, rounding, alpha, recycling
gray(c(0, 0.5, 1))
gray(c(0.1, 0.25, 1/3, 0.99))
gray(0.5, alpha = 0.5)
gray(c(0, 1), alpha = 0.5)
gray(numeric(0))
grey(0.75)

# rgb: [0,1], 0:255 and other maxColorValue, names, alpha
rgb(1, 0, 0)
rgb(c(1, 0), c(0, 1), 0)
rgb(255, 128, 0, maxColorValue = 255)
rgb(10, 20, 30, 40, maxColorValue = 255)
rgb(0.5, 0.25, 0.75, alpha = 0.5)
rgb(5, 5, 5, maxColorValue = 10)
rgb(1, 0, 0, names = "pure red")
rgb(numeric(0), 0, 0)

# hsv: the six hue sectors, alpha, recycling
hsv(c(0, 1/6, 1/3, 0.5, 2/3, 5/6, 1), 1, 1)
hsv(0.5, 0.5, 0.5)
hsv(0, 0, c(0, 0.5, 1))
hsv(0.3, 0.7, 0.9, alpha = 0.4)

# hcl: in gamut, out of gamut with and without fixup, NA, alpha
hcl(c(0, 90, 180, 270), 50, 50)
hcl(120, 40, 60)
hcl(0, 200, 50)
hcl(0, 200, 50, fixup = FALSE)
hcl(c(NA, 0), 50, 50)
hcl(240, 30, 70, alpha = 0.5)
hcl(240, 30, 70, alpha = NA)

# rgb2hsv over col2rgb output (the documented round trip; the R
# wrapper divides by maxColorValue itself)
rgb2hsv(col2rgb(c("red", "green", "blue", "white", "black", "gray50")))
rgb2hsv(matrix(c(0.2, 0.4, 0.6), nrow = 3,
               dimnames = list(NULL, "one")), maxColorValue = 1)

# every error message
for (e in list(
  function() gray(c(0, 1), alpha = c(0, 0.5, 1)),
  function() gray(0.5, alpha = c(0.25, 0.75)),
  function() gray(1.5),
  function() gray(NA_real_),
  function() gray(0.5, alpha = 2),
  function() gray(0.5, alpha = NA_real_),
  function() rgb(100000, 0, 0, maxColorValue = 1),
  function() rgb(999999.5, 0, 0, maxColorValue = 1),
  function() rgb(-9.999995e-05, 0, 0),
  function() rgb(2, 0, 0),
  function() rgb(NA_real_, 0, 0),
  function() rgb(0, 0, 0, alpha = -0.5),
  function() rgb(300, 0, 0, maxColorValue = 255),
  function() rgb(NA_integer_, 0, 0, maxColorValue = 255),
  function() rgb(0, 0, 0, alpha = 300, maxColorValue = 255),
  function() rgb(0, 0, 0, maxColorValue = 0),
  function() rgb(1, 0, 0, names = c("a", "b")),
  function() hsv(2, 1, 1),
  function() hsv(0.5, NA_real_, 1),
  function() hsv(0.5, 1, 1, alpha = 1.5),
  function() hcl(0, -1, 50),
  function() hcl(0, 50, 101),
  function() hcl(0, 50, 50, alpha = 2)
)) print(tryCatch(e(), error = function(x) conditionMessage(x)))
