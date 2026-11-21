# The system profile's Autoloads environment (profile/Common.R, run by
# R_LoadProfile before base is locked): attached at position 2, bound in
# base as .AutoloadEnv, holding .Autoloaded; the search path after other
# attaches, as.environment() and pos.to.env() of it, and its detach.
print(search())
print(environmentName(.AutoloadEnv))
print(identical(.AutoloadEnv, as.environment(2)))
print(identical(.AutoloadEnv, as.environment("Autoloads")))
print(identical(.AutoloadEnv, pos.to.env(2)))
print(ls(.AutoloadEnv, all.names = TRUE))
print(.Autoloaded)
print(attributes(.AutoloadEnv))
print(environmentName(parent.env(globalenv())))
print(identical(parent.env(.AutoloadEnv), baseenv()))
print(bindingIsLocked(".AutoloadEnv", baseenv()))
x <- attach(list(a = 1), name = "L")
print(search())
print(identical(.AutoloadEnv, as.environment(3)))
detach("L")
detach("Autoloads")
print(search())
print(environmentName(.AutoloadEnv))
