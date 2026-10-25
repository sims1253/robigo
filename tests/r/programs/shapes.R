area <- function(s) UseMethod("area")
area.circle <- function(s) pi * s$r^2
area.rect <- function(s) s$w * s$h
area.default <- function(s) stop("unknown shape")
print.shape <- function(x, ...) {
  cat(sprintf("<%s with area %.2f>\n", class(x)[1], area(x)))
  invisible(x)
}
circle <- function(r) structure(list(r = r), class = c("circle", "shape"))
rect <- function(w, h) structure(list(w = w, h = h), class = c("rect", "shape"))
shapes <- list(circle(1), rect(2, 3), circle(0.5))
for (s in shapes) print(s)
areas <- vapply(shapes, area, numeric(1))
round(areas, 3)
total <- sum(areas)
sprintf("total area: %.4f", total)
largest <- shapes[[which.max(areas)]]
class(largest)
