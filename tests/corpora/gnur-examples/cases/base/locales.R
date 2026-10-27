RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: locales
### Title: Query or Set Aspects of the Locale
### Aliases: locales Sys.getlocale Sys.setlocale .LC.categories LC_ALL
###   LC_COLLATE LC_CTYPE LC_MONETARY LC_NUMERIC LC_TIME LC_MESSAGES
###   LC_PAPER LC_MEASUREMENT
### Keywords: utilities

### ** Examples

Sys.getlocale()

## Date-time  related :
Sys.getlocale("LC_TIME") -> olcT
then <- as.POSIXlt("2001-01-01 01:01:01", tz = "UTC")
## Not run: 
##D c(m = months(then), wd = weekdays(then)) # locale specific
##D Sys.setlocale("LC_TIME", "de")     # Solaris: details are OS-dependent
##D Sys.setlocale("LC_TIME", "de_DE")  # Many Unix-alikes
##D Sys.setlocale("LC_TIME", "de_DE.UTF-8")  # Linux, macOS, other Unix-alikes
##D Sys.setlocale("LC_TIME", "de_DE.utf8")   # some Linux versions
##D Sys.setlocale("LC_TIME", "German.UTF-8") # Windows
##D Sys.getlocale("LC_TIME") # the last one successfully set above
##D c(m = months(then), wd = weekdays(then)) # in C_TIME locale 'cT' ; typically German
## End(Not run)
Sys.setlocale("LC_TIME", "C")
c(m = months(then), wd = weekdays(then)) # "standard" (still platform specific ?)
Sys.setlocale("LC_TIME", olcT)           # reset to previous

## Other locales
Sys.getlocale("LC_PAPER")          # may or may not be set
.LC.categories # of length 9 on all platforms

## Not run: 
##D Sys.setlocale("LC_COLLATE", "C")   # turn off locale-specific sorting,
##D                                    # usually (but not on all platforms)
##D Sys.setenv("LANGUAGE" = "es") # set the language for error/warning messages
## End(Not run)
## No test: 
##D ## some nice formatting; should work on most platforms,
##D           ## macOS does not name the entries.
##D  sep <- switch(Sys.info()[["sysname"]],
##D                "Darwin"=, "SunOS" = "/",
##D                "Linux" =, "Windows" = ";")
##D  ##' named vector from a "full" Sys.getlocale() :
##D  asNvec <- function(loc) {
##D      sl <- strsplit(strsplit(loc, sep)[[1L]], "=")
##D      if(all(lengths(sl) == 2L))
##D         setNames(sapply(sl, `[[`, 2L), sapply(sl, `[[`, 1L))
##D      else
##D        setNames(as.character(sl), .LC.categories[1+seq_along(sl)])
##D  }
##D  print.Dlist(lloc <- asNvec(Sys.getlocale()))
##D  ## R-supported ones (but LC_ALL):
##D  lloc[.LC.categories[-1]]
## End(No test)



