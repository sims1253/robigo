RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: userhooks
### Title: Functions to Get and Set Hooks for Load, Attach, Detach and
###   Unload
### Aliases: getHook setHook packageEvent .userHooksEnv
### Keywords: utilities

### ** Examples

setHook(packageEvent("grDevices", "onLoad"),
        function(...) grDevices::ps.options(horizontal = FALSE))



