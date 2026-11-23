# INTERIM(S1): closes when robigo-rt's deparse can force (it reads with
# &Cx, deparse.rs's module doc). R: deparse.c:deparse2buff, the call's
# function: a symbol whose base binding (SYMVALUE) is a promise is
# evaluated with eval(val, R_BaseEnv) before its kind is read, so the
# promise's errors and effects happen. robigo-rt's sym_prim reads the
# binding as it is.
unlockBinding("identity", baseenv())
delayedAssign("identity", stop("boom"), assign.env = baseenv())
print(tryCatch(deparse(quote(identity(1))), error = conditionMessage))
