library(methods)
# R: methods_list_dispatch.c:R_set_method_dispatch -> coerce.c:asBool
tryCatch(.Call(methods:::C_R_set_method_dispatch, c(TRUE, FALSE)),
         error = function(e) cat("E:", conditionMessage(e), "\n"))
tryCatch(.Call(methods:::C_R_set_method_dispatch, integer(0)),
         error = function(e) cat("E:", conditionMessage(e), "\n"))
tryCatch(.Call(methods:::C_R_set_method_dispatch, NULL),
         error = function(e) cat("E:", conditionMessage(e), "\n"))
