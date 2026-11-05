# grDevices' cairo devices: GNU R loads them from the cairo module,
# R_HOME/library/grDevices/libs/cairo.so (R: Rdynload.c:R_cairoCdynload,
# grDevices/src/devCairo.c), built from src/library/grDevices/src/cairo.
print(capabilities(c("jpeg", "png", "tiff", "cairo")))
dev <- function(open, ext) {
  f <- tempfile(fileext = ext)
  open(f)
  graphics::plot(1:10, main = "cairo")
  grDevices::dev.off()
  c(exists = file.exists(f), nonempty = file.size(f) > 0)
}
print(dev(function(f) grDevices::png(f, width = 200, height = 200), ".png"))
print(dev(function(f) grDevices::jpeg(f, width = 200, height = 200), ".jpg"))
print(dev(function(f) grDevices::tiff(f, width = 200, height = 200), ".tiff"))
print(dev(function(f) grDevices::bmp(f, width = 200, height = 200), ".bmp"))
print(dev(grDevices::svg, ".svg"))
print(dev(grDevices::cairo_pdf, ".pdf"))
print(dev(grDevices::cairo_ps, ".ps"))
# what each file starts with
f <- tempfile(fileext = ".png")
grDevices::png(f, width = 300, height = 300)
graphics::plot.new()
invisible(grDevices::dev.off())
print(readBin(f, "raw", 8))
f <- tempfile(fileext = ".svg")
grDevices::svg(f, width = 4, height = 4)
graphics::plot.new()
invisible(grDevices::dev.off())
print(readLines(f, n = 1))
print(getOption("bitmapType"))
print("cairo" %in% names(getLoadedDLLs()))
