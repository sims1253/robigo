# library.dynam() of tools from R_HOME (library/tools/libs/tools.so, which
# scripts/rt-home.sh builds from GNU R's sources against robigo's headers;
# A5): the package's DLL found under .Library, loaded and recorded in
# .dynLibs(), its routines registered by R_init_tools.
d <- library.dynam("tools", "tools", .Library)
class(d)
d[["name"]]
basename(d[["path"]])
d[["dynamicLookup"]]
d[["forceSymbols"]]
vapply(.dynLibs(), function(x) x[["name"]], "")
sapply(getDLLRegisteredRoutines(d), length)
