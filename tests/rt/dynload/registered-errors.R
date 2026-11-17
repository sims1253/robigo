# Rdynload.c's errors for getRegisteredRoutines and getLoadedDLLs.
tryCatch(.Internal(getRegisteredRoutines(getLoadedDLLs()[["base"]][["handle"]])),
         error = function(e) conditionMessage(e))
tryCatch(.Internal(getRegisteredRoutines(1)), error = function(e) conditionMessage(e))
tryCatch(.Internal(getRegisteredRoutines(NULL)), error = function(e) conditionMessage(e))
tryCatch(.Internal(getLoadedDLLs(1)), error = function(e) conditionMessage(e))
tryCatch(getDLLRegisteredRoutines("nosuchdll"), error = function(e) conditionMessage(e))
tryCatch(getDLLRegisteredRoutines.DLLInfo(1), error = function(e) conditionMessage(e))
.Internal(getRegisteredRoutines(1))
