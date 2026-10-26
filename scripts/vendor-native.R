## Dump the native routines that GNU R's base packages register, for
## rlib/native-routines.tsv: robigo does not load the packages' shared
## libraries, but reports these registrations from dyn.load() so that
## loadNamespace() binds the same C_xxx symbols; calls dispatch to robigo's
## ports. Run with: Rscript scripts/vendor-native.R > rlib/native-routines.tsv
pkgs <- c("base", "utils", "stats", "methods", "grDevices", "graphics",
          "tools", "parallel", "grid", "splines", "compiler", "tcltk",
          "stats4", "datasets")
cat("# package\ttype\tname\tnumParameters (type DLL: dynamicLookup, forceSymbols)\n")
for (p in pkgs) {
    if (p != "base") suppressWarnings(try(loadNamespace(p), silent = TRUE))
    dlls <- getLoadedDLLs()
    if (!p %in% names(dlls)) next
    d <- dlls[[p]]
    cat(p, "\tDLL\t", d[["dynamicLookup"]], "\t", d[["forceSymbols"]], "\n", sep = "")
    r <- getDLLRegisteredRoutines(d)
    for (type in names(r))
        for (s in r[[type]])
            cat(p, "\t", type, "\t", s$name, "\t", s$numParameters, "\n", sep = "")
}
