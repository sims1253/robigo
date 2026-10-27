### Name: sourceutils
### Title: Source Reference Utilities
### Aliases: getSrcFilename getSrcDirectory getSrcref getSrcLocation
### Keywords: utilities

### ** Examples

fn <- function(x) {
  x + 1 # A comment, kept as part of the source
}			

# Show the temporary file directory
# where the example was saved

getSrcDirectory(fn)
getSrcLocation(fn, "line")



