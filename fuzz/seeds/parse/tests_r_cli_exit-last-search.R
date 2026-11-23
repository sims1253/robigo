e <- new.env()
e$.Last <- function() cat('attached-last\n')
attach(e, name = 'shutdown-test')
