### Name: aspell
### Title: Spell Check Interface
### Aliases: aspell
### Keywords: utilities

### ** Examples

## Not run: 
##D ## To check all Rd files in a directory, (additionally) skipping the
##D ## \references sections.
##D files <- Sys.glob("*.Rd")
##D aspell(files, filter = list("Rd", drop = "\\references"))
##D 
##D ## To check all Sweave files
##D files <- Sys.glob(c("*.Rnw", "*.Snw", "*.rnw", "*.snw"))
##D aspell(files, filter = "Sweave", control = "-t")
##D 
##D ## To check all Texinfo files (Aspell only)
##D files <- Sys.glob("*.texi")
##D aspell(files, control = "--mode=texinfo")
## End(Not run)

## List the available R system dictionaries.
Sys.glob(file.path(R.home("share"), "dictionaries", "*.rds"))



