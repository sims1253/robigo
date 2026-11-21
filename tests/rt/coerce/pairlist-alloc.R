# coercePairList (coerce.c) of NULL or a pairlist to a type allocVector cannot
# make: allocVector3 fails first ("invalid type/length"), before the
# switch's UNIMPLEMENTED_TYPE; `[<-`(NULL, , ) gets there with the
# missing value (a symbol) as the value (subassign.c:do_subassign_dflt)
m <- function(e) tryCatch({ e; "ok" }, error = function(c) conditionMessage(c))
print(m(`[<-`(NULL, , )))
print(m(`[<-`(NULL, 1, )))
print(m(`[<-`(NULL, , , )))
print(m(as.vector(NULL, "symbol")))
print(m(as.vector(pairlist(1, 2), "symbol")))
print(m(as.vector(pairlist(1, 2), "environment")))
print(m(as.vector(NULL, "pairlist")))
print(m(as.vector(pairlist(1), "list")))
f <- function() `[<-`(NULL, , )
print(tryCatch(f(), error = function(c) conditionCall(c)))
x <- NULL
print(m(x[[1]] <- quote(a)))
print(m({x <- NULL; x[1] <- list(quote(a)); x}))
