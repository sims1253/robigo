# Force every lazy-load binding of every base package, so the recording
# battery (scripts/build-rdbflat.sh) logs a fetch for every entry of
# every base package's .rdb and scripts/build-rdbflat.py writes a
# COMPLETE flat cache: any later run serves all its lazyLoadDBfetch
# calls from ROBIGO_RDBFLAT instead of falling back to the per-fetch
# inflate path.
#
# Forcing a name evaluates its promise (a lazyLoadDBfetch call); entries
# only reachable through reference environments are fetched while the
# promising object unserializes, so the transitive closure of forcing
# every binding covers every fetch any program can make.
force_all <- function(env) for (n in ls(env, all.names = TRUE))
    tryCatch(get(n, envir = env), error = function(e) NULL)

force_all(baseenv())
for (p in c("compiler", "datasets", "grDevices", "graphics", "grid",
            "methods", "parallel", "splines", "stats", "stats4",
            "tcltk", "tools", "utils")) {
    ok <- tryCatch({ library(p, character.only = TRUE); TRUE },
                   error = function(e) FALSE)
    if (ok) {
        force_all(as.environment(paste0("package:", p)))
        force_all(getNamespace(p))
    }
}
