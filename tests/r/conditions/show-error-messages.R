# R_ShowErrorMessages (the show.error.messages option) gates the error
# printing, not the unwinding: errors.c:verrorcall_dflt. rlang's
# signal_abort() prints its own styled error and stop()s with the option
# off; without this, uncaught rlang errors print twice.
options(show.error.messages = FALSE)
stop("hidden message")
print("not reached")
