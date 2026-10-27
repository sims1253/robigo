RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getDLLRegisteredRoutines
### Title: Reflectance Information for C/Fortran routines in a DLL
### Aliases: getDLLRegisteredRoutines getDLLRegisteredRoutines.character
###   getDLLRegisteredRoutines.DLLInfo print.NativeRoutineList
###   print.DLLRegisteredRoutines
### Keywords: interface

### ** Examples

dlls <- getLoadedDLLs()
getDLLRegisteredRoutines(dlls[["base"]])

getDLLRegisteredRoutines("stats")



