# needs: ggplot2
# Building (not drawing) a plot: S7 object construction, layer data,
# scales and facet layout, all printable without a device.
suppressPackageStartupMessages(library(ggplot2))
df <- data.frame(x = 1:4, y = c(2.1, 3.9, 6.2, 7.8), g = c("a", "a", "b", "b"))
p <- ggplot(df, aes(x, y, colour = g)) + geom_point() + geom_smooth(method = "lm", se = FALSE) +
  facet_wrap(~g) + labs(title = "smoke")
b <- ggplot_build(p)
cat("layers:", length(b$data), " panels:", nrow(b$layout$layout), "\n")
print(b$data[[1]][c("x", "y", "colour")])
print(layer_scales(p)$x$get_limits())
