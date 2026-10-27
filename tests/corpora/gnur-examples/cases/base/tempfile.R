### Name: tempfile
### Title: Create Names for Temporary Files
### Aliases: tempfile tempdir
### Keywords: file

### ** Examples
## No test: 
##D tempfile(c("ab", "a b c"))   # give file name with spaces in!
##D 
##D tempfile("plot", fileext = c(".ps", ".pdf"))
##D 
##D tempdir() # works on all platforms with a platform-dependent result
## End(No test)

## Show how 'check' is working on some platforms:
if(exists("I'm brave") && `I'm brave` &&
   identical(.Platform$OS.type, "unix") && grepl("^/tmp/", tempdir())) {
  cat("Current tempdir(): ", tempdir(), "\n")
  cat("Removing it :", file.remove(tempdir()),
      "; dir.exists(tempdir()):", dir.exists(tempdir()), "\n")
  cat("and now  tempdir(check = TRUE) :", tempdir(check = TRUE),"\n")
}




