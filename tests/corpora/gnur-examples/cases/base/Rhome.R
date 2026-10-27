### Name: Rhome
### Title: Return the R Home Directory
### Aliases: R.home R_HOME
### Keywords: utilities

### ** Examples

## These result quite platform-dependently :
rbind(home = R.home(),
      bin  = R.home("bin")) # often the 'bin' sub directory of 'home'
                            # but not always ...
list.files(R.home("bin"))



