# The capi corpus's pairlist case (tests/corpora/capi/cases/pairlist.R) for
# robigo-rt, its C in support/pairlist.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS: the corpus case's support file without
# capi_car_as_call, whose SET_TYPEOF is not in GNU R 4.6's API (the
# foreman's answer 2).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/pairlist.so"))

x <- .Call("capi_cons")
print(x[[1]])           # the tagged pairlist
for (e in x[-1]) print(e)

call <- quote(x + y)
print(.Call("capi_cons_mutate", call))
print(call)             # the original call is not mutated

l <- .Call("capi_langs")
print(l)
print(l[[6]])
print(eval(l[[6]]))

print(.Call("capi_convert", list(TRUE, 2L, "three")))
