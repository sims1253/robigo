dyn.load("pairlist.so")

# SETCAR on a shared pairlist: every binding sees the new cell.
p <- pairlist(1, 2, 3)
q <- p
r <- .Call("capi_setcar_alias", p)
print(p)
print(q)
print(r)
print(identical(p, q))
print(identical(p, r))

# A cell the routine did not replace keeps its object identity: q's first
# cell still aliases `inner`, so mutating `inner` is visible through q.
inner <- pairlist(10)
p <- pairlist(inner, 2)
q <- p
.Call("capi_setcar2_alias", p)
.Call("capi_setcar_alias", inner)
print(q[[1]])
print(q)

# SETCDR splices the tail of the caller's spine.
z <- pairlist(1, 2, 3)
w <- z
.Call("capi_setcdr_alias", z, pairlist(8, 9))
print(z)
print(w)
print(length(w))

# SET_TAG: names are cons-cell tags, mutated in place.
a <- pairlist(x = 1, 2)
b <- a
.Call("capi_settag_alias", a, as.name("zz"))
print(names(a))
print(names(b))

# LANGSXP: cell 0 is the function, the rest are the arguments.
l <- quote(f(a, b))
m <- l
.Call("capi_setfun_alias", l)
print(l)
print(m)

call <- quote(f(a, b))
call_alias <- call
.Call("capi_setcdr_alias", call, pairlist(7))
print(call)
print(call_alias)

# A cell a routine returns is the caller's own: writes through the result
# are seen through the list it came from, in R and in later routines.
outer <- pairlist(pairlist(FALSE, 0L), 2)
status <- .Call("capi_car", outer)
.Call("capi_setcar2_alias", status)
print(status)
print(outer[[1]])
print(identical(.Call("capi_car", outer), status))
.Call("capi_setcar2_alias", .Call("capi_car", outer))
print(outer)
e <- new.env()
e$outer <- outer
.Call("capi_setcar_alias", .Call("capi_car", e$outer))
print(e$outer[[1]])

# The same for a call inside a call.
l <- quote(f(g(a), h(b)))
inner <- .Call("capi_cadr", l)
.Call("capi_setfun_alias", inner)
print(inner)
print(l)

r <- .Call("capi_car_as_call", pairlist(pairlist(as.name("f"), 1)))
print(typeof(r))
print(r)
