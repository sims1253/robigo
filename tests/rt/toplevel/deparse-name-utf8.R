# An error report names its call as deparse does (quotify: backquotes
# and escapes only for a non-syntactic name).
`é` <- function() nope
`é`()
