### Name: DLL.version
### Title: DLL Version Information on MS Windows
### Aliases: DLL.version
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "windows") withAutoprint({
  DLL.version(file.path(R.home("bin"), "R.dll"))
  DLL.version(file.path(R.home(), "library/stats/libs", .Platform$r_arch, "stats.dll"))
})



