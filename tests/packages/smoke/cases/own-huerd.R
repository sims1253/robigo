# needs: huerd
# The user's own package (github.com/sims1253/huerd): colour palettes
# optimised for colour vision deficiency. Its README's basic usage,
# with 4 colours instead of 8 to stay in seconds.
suppressPackageStartupMessages(library(huerd))
set.seed(42)
palette <- generate_palette(4, progress = FALSE)
print(palette)
print(quick_palette(4))
