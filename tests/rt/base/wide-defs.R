# INTERIM(L5): the wide base's definitions that are not the last
# assignment alone. The inner names of a chain (`licence <- license <-`),
# which M1's loader dropped:
identical(license, licence)
identical(numeric, double)
c(is.function(objects), is.function(`[.simple.list`))
# assignments whose value is a primitive (zzz.R and baseloader.R)
c(is.primitive(as.numeric), is.primitive(is.name))
# the forms after a definition: zzz.R's assign()s into .ArgsEnv, and
# baseloader.R's lock of it
length(ls(.ArgsEnv, all.names = TRUE))
bindingIsLocked("list", .ArgsEnv)
# the profile's (R_LoadProfile evaluates it in base) and GNU R's C
T; F
.sys.timezone
is.function(.First.sys)
.Device
.Devices
# a value computed from base's own definitions
month.abb
