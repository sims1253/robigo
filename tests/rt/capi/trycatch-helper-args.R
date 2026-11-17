# .Internal(C_tryCatchHelper(...)) called from R with too few arguments or
# no external pointer: CAR of a short argument list is R_NilValue, and the
# helper's type check raises the error
tryCatch(.Internal(C_tryCatchHelper()), error = function(e) conditionMessage(e))
tryCatch(.Internal(C_tryCatchHelper(1, 0)), error = function(e) conditionMessage(e))
tryCatch(.Internal(C_tryCatchHelper(NULL, 1, NULL)), error = function(e) conditionCall(e))
.Internal(C_tryCatchHelper())
