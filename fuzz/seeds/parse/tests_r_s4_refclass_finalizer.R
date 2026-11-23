library(methods)
clazz <- suppressMessages(setRefClass('Foo18R5', fields=c('a'), methods=list(initialize=function() a <<- 456, finalize=function() print(sprintf('finalizer: %d', a)))))
(function() { x <- clazz$new(); print('fun') })()
invisible(gc())
