# Sys.setenv / Sys.unsetenv (sysutils.c do_setenv, do_unsetenv)
Sys.setenv(ROBIGO_T_A = "one")
print(Sys.setenv(ROBIGO_T_A = "two", ROBIGO_T_B = 3.5))
Sys.getenv(c("ROBIGO_T_A", "ROBIGO_T_B"))
Sys.setenv(ROBIGO_T_C = NA, ROBIGO_T_D = TRUE)
Sys.getenv(c("ROBIGO_T_C", "ROBIGO_T_D"))
Sys.setenv(ROBIGO_T_E = "")
Sys.getenv("ROBIGO_T_E", unset = NA)
Sys.setenv(ROBIGO_T_A = "x")  # invisible
# names setenv refuses
print(.Internal(Sys.setenv(c("ROBIGO_T_F", "A=B", ""), c("f", "g", "h"))))
Sys.getenv("ROBIGO_T_F")
# getenv("A=B") matches the entry A=B=C, as glibc's does
Sys.setenv(ROBIGO_T_G = "B=C")
print(.Internal(Sys.unsetenv("ROBIGO_T_G=B")))
Sys.getenv("ROBIGO_T_G")
# unset
print(Sys.unsetenv(c("ROBIGO_T_A", "ROBIGO_T_B", "ROBIGO_T_NOPE", "")))
Sys.getenv(c("ROBIGO_T_A", "ROBIGO_T_B"), unset = NA)
Sys.unsetenv("ROBIGO_T_C")  # invisible
print(Sys.unsetenv(character(0)))
print(.Internal(Sys.setenv(character(0), character(0))))
print(Sys.unsetenv(1:2))
# a latin1 name and value are set in the native encoding
x <- "ROBIGO_T_\xe9"; Encoding(x) <- "latin1"
v <- "caf\xe9"; Encoding(v) <- "latin1"
print(.Internal(Sys.setenv(x, v)))
Sys.getenv("ROBIGO_T_é")
print(Sys.unsetenv(x))
Sys.getenv("ROBIGO_T_é", unset = NA)
# errors
tryCatch(Sys.setenv("a"), error = function(e) print(conditionMessage(e)))
tryCatch(.Internal(Sys.setenv(1, "a")), error = function(e) print(conditionMessage(e)))
tryCatch(.Internal(Sys.setenv("a", 1)), error = function(e) print(conditionMessage(e)))
tryCatch(.Internal(Sys.setenv(c("a", "b"), "a")), error = function(e) print(conditionMessage(e)))
tryCatch(.Internal(Sys.unsetenv(1)), error = function(e) print(conditionMessage(e)))
f <- function(n, v) .Internal(Sys.setenv(n, v))
tryCatch(f(1, "a"), error = function(e) print(conditionCall(e)))
b <- "ROBIGO_T_\xff"; Encoding(b) <- "bytes"
tryCatch(.Internal(Sys.setenv(c("ROBIGO_T_H", b), c("h", "i"))), error = function(e) print(conditionMessage(e)))
Sys.getenv("ROBIGO_T_H")
tryCatch(Sys.unsetenv(b), error = function(e) print(conditionMessage(e)))
