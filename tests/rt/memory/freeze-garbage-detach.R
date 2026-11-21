# The boot heap's freeze: a package environment attached at the freeze
# (Autoloads, attached by the system profile) given a finalizable
# environment after it, then detached: the detached environment and what
# it holds die at the next gc(), as in GNU R. (base binds Autoloads as
# .AutoloadEnv too: that binding is rebound first.)
e <- new.env()
reg.finalizer(e, function(x) cat("finalized\n"))
assign("held", e, envir = as.environment("Autoloads"))
rm(e)
invisible(gc())
cat("held by Autoloads\n")
unlockBinding(".AutoloadEnv", baseenv())
assign(".AutoloadEnv", NULL, envir = baseenv())
detach("Autoloads")
1  # (.Last.value no longer the detached environment)
invisible(gc())
cat("after detach\n")
print(search())
