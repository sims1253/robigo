# .Internal(internalsID()) (version.c's do_internalsID): R_INTERNALS_UUID;
# it checks no arguments, so extra ones are ignored.
id <- .Internal(internalsID())
print(id)
print(identical(.Internal(internalsID(1)), id))
print(identical(.Internal(internalsID(1, "a", NULL)), id))
