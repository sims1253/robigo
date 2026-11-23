# The boot heap's freeze: a finalizer put on a boot environment (Autoloads,
# attached by the system profile), which base then drops: detached, and
# its .AutoloadEnv binding rebound. It dies at the next gc() and its
# finalizer runs, as in GNU R. (With ROBIGO_GC_FREEZE=1 the finalizer
# queues the frozen environment to unfreeze; the detach cuts it off the
# search path, and the frozen-reachability pass lets it die.)
# (with ROBIGO_GC_FREEZE=1 the freeze is the first collection after the
# boot: this one, so that what follows acts on frozen objects)
invisible(gc())
a <- as.environment("Autoloads")
reg.finalizer(a, function(x) cat("Autoloads finalized\n"))
rm(a)
invisible(gc())
cat("still attached\n")
unlockBinding(".AutoloadEnv", baseenv())
assign(".AutoloadEnv", NULL, envir = baseenv())
detach("Autoloads")
1  # (.Last.value no longer the detached environment)
invisible(gc())
cat("after detach\n")
print(search())
