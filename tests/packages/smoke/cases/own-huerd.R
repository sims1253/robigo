# needs: huerd
# The user's own package (github.com/sims1253/huerd): perceptual
# color-palette optimization in OKLAB. Exercises a small deterministic
# palette generation.
suppressPackageStartupMessages(library(huerd))
pal <- generate_palette(4, progress = FALSE)
print(length(pal))
