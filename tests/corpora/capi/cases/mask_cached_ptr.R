dyn.load("mask_cached_ptr.so")

# The mask loop: a duplicated scalar group id (dplyr binds
# duplicate(0L)), rewritten through a cached INTEGER() pointer
# between evaluations of an active binding that subsets by it.
info <- new.env(parent = baseenv())
info[["pkg:::current_group_id"]] <- .Call("capi_fresh_zero")
fenv <- new.env(parent = info)
fenv$chops <- list(10:11, 20:22)
f <- function() .subset2(chops, `pkg:::current_group_id`)
environment(f) <- fenv
mask <- new.env(parent = info)
makeActiveBinding("colx", f, mask)
print(.Call("capi_mask_loop", info, mask))
print(info[["pkg:::current_group_id"]])

# Attribute-pairlist identity: boxes C made are still boxes after a
# trip through R, a list round trip, and a pairlist->list coercion.
.Call("capi_box_init")
b1 <- .Call("capi_box_make", list("a"))
print(.Call("capi_box_is", b1))
pair <- .Call("capi_box_pair")
print(.Call("capi_box_is", pair[[1]]))
print(.Call("capi_box_is", pair[[2]]))
lst <- list(b1, pair[[1]])
print(.Call("capi_box_is", lst[[2]]))
