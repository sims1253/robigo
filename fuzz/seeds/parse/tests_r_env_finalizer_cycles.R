events <- character()
fin <- function(e) events <<- c(events, e$id)
probe <- function(label, f) {
    events <<- character()
    f()
    invisible(gc())
    cat(label, ': ', paste(events, collapse=','), '\n', sep='')
}
probe('plain', function() { e <- new.env(); e$id <- 'plain'; reg.finalizer(e, fin) })
probe('self', function() { e <- new.env(); e$id <- 'self'; e$self <- e; reg.finalizer(e, fin) })
probe('mutual', function() {
    a <- new.env(); b <- new.env(); a$b <- b; b$a <- a
    a$id <- 'mutual'; reg.finalizer(a, fin)
})
probe('closure', function() {
    e <- new.env(); e$id <- 'closure'; e$f <- function() e
    reg.finalizer(e, fin)
})
probe('attribute', function() {
    e <- new.env(); e$id <- 'attribute'; attr(e, 'self') <- e
    reg.finalizer(e, fin)
})
probe('shared', function() {
    e <- new.env(); e$id <- 'shared'; e$self <- e
    reg.finalizer(e, fin); keeper <<- list(e)
    invisible(gc()); stopifnot(length(events) == 0)
    rm(keeper, envir=.GlobalEnv)
})
