# The C and Fortran routines baseloader.R binds in base from
# getDLLRegisteredRoutines("base") (R_init_base's, registration.c).
r <- getDLLRegisteredRoutines("base")
class(r)
vapply(r, length, 0L)
class(.C_R_addTaskCallback)
.C_R_addTaskCallback$name
.C_R_getTaskCallbackNames$name
.C_R_removeTaskCallback$numParameters
.F_dqrdc2$name
.F_dtrco$numParameters
class(.F_dqrcf)
vapply(c(".F_dqrcf", ".F_dqrdc2", ".F_dqrqty", ".F_dqrqy", ".F_dqrrsd", ".F_dqrxb", ".F_dtrco"),
       function(n) get(n)$numParameters, 0L)
getDLLRegisteredRoutines.character("base")$.Fortran$dqrxb$name
