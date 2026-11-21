# The boot heap's freeze (robigo-heap's freeze.rs) keeps what R sees of a
# finalizer on a boot object: as GNU R, a finalizer on an environment base
# made at boot, and one on base itself (which runs at exit).
# a finalizer on an environment base made at boot, its base binding then
# replaced: base still holds it elsewhere, so neither GNU R nor robigo-rt
# finalizes it at gc() (robigo-rt unfreezes it at that collection)
e <- get(".userHooksEnv", baseenv())
print(is.environment(e))
reg.finalizer(e, function(e) cat("finalized base's .userHooksEnv\n"))
rm(e)
unlockBinding(".userHooksEnv", baseenv())
assign(".userHooksEnv", new.env(hash = FALSE, parent = baseenv()), envir = baseenv())
lockBinding(".userHooksEnv", baseenv())
invisible(gc())
cat("after gc\n")
# a finalizer on base's environment itself: base never dies, so it runs at exit
reg.finalizer(baseenv(), function(e) cat("base finalized at exit\n"), onexit = TRUE)
cat("end\n")
