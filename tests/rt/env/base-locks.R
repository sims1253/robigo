# setup_Rmainloop locks base's namespace with its bindings, and baseenv(),
# right after the system profile; .Device and .Devices are unlocked again
# for graphics (main.c). The stores GNU R's C makes into base go past the
# locks (SET_SYMVALUE): .Last.value, last.warning, the option list.
print(bindingIsLocked("paste", baseenv()))
print(bindingIsLocked("paste", .BaseNamespaceEnv))
print(bindingIsLocked("T", baseenv()))
print(bindingIsLocked(".Device", baseenv()))
print(bindingIsLocked(".Devices", baseenv()))
print(tryCatch(assign("zzz", 1, envir = baseenv()), error = conditionMessage))
print(tryCatch(assign("paste", 1, envir = baseenv()), error = conditionMessage))
print(tryCatch(assign("paste", 1, envir = .BaseNamespaceEnv), error = conditionMessage))
f <- function() paste0("a", "b")
print(f())
invisible(5)
print(.Last.value)
options(robigo.base.locks = 1)
print(getOption("robigo.base.locks"))
g <- function() {
    warning("from g")
    2
}
x <- g()
print(names(last.warning))
