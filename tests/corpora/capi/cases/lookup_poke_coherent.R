dyn.load("lookup_poke.so")

# rlang's deparser: a stack of (reset, n_sticky) pairlists in an object
self <- new.env()
self$indent_status <- pairlist(pairlist(FALSE, 0L), pairlist(TRUE, 7L))
for (i in 1:3) {
    status <- .Call("capi_poke_car", self$indent_status)
    .Call("capi_poke_set_cadr", status, .Call("capi_poke_cadr", status) + 1L)
}
status <- .Call("capi_poke_car", self$indent_status)
.Call("capi_poke_set_car", status, TRUE)
print(.Call("capi_poke_car", self$indent_status))
print(.Call("capi_poke_cadr", self$indent_status))

# a list inside a list
outer <- list(list(1, 2), "b")
inner <- .Call("capi_poke_elt", outer, 0L)
.Call("capi_poke_set_elt", inner, 1L, "two")
print(outer[[1]][[2]])
print(inner[[2]])

# through a variable C looks up
e <- new.env()
e$state <- list(list("a", "b"))
invisible(.Call("capi_poke_lookup", e, "B"))
print(e$state[[1]][[2]])
