# The boot heap's freeze (robigo-heap's freeze.rs) keeps what R sees of a
# finalizer on a boot object: as GNU R, a finalizer on an environment base
# made at boot, and one on base itself (which runs at exit).
# a finalizer on an environment base binds: base still holds it, so
# neither GNU R nor robigo-rt finalizes it at gc() (robigo-rt makes it, and
# any frozen object that reaches it, ordinary at that collection)
e <- get(".userHooksEnv", baseenv())
print(is.environment(e))
reg.finalizer(e, function(e) cat("finalized base's .userHooksEnv\n"))
rm(e)
invisible(gc())
print(is.environment(get(".userHooksEnv", baseenv())))
cat("after gc\n")
# a finalizer on base's environment itself: base never dies, so it runs at exit
reg.finalizer(baseenv(), function(e) cat("base finalized at exit\n"), onexit = TRUE)
cat("end\n")
