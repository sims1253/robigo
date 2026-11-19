# Known difference (not ratcheted until V4 step 3 installs the JIT's byte
# code as BODY): a closure the JIT compiled, serialized after a base
# function it calls was shadowed. GNU R writes the code its JIT made before
# the shadow (`length` inlined behind a base guard), so the bytes are the
# same before and after; robigo-rt compiles at the write (R_cmpfun1), after
# the shadow, and writes a generic call. It cannot occur in base's build
# (base's environment is not rebound between a JIT moment and makebasedb's
# write); in a package build only if its own top-level code calls a closure
# and then shadows a base function the closure uses.
f <- function(x) { for (i in 1) NULL; length(x) + 1L }
f(1:3)
a <- serialize(f, NULL)
length <- function(x) 99L
b <- serialize(f, NULL)
cat(identical(a, b), base::length(a), base::length(b), "\n")
print(f(1:3))
