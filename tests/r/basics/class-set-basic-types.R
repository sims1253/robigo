# class<- with a basic type (R: coerce.c:R_set_class and classTable):
# "function", "NULL", "call", ... are ordinary S3 classes; "name",
# "environment", "externalptr", "weakref" check the type; the vector types
# coerce through ascommon, which reports the setter call for non-vectors.
for (k in c("function", "NULL", "call", "language", "symbol", "name",
            "environment", "externalptr", "weakref")) {
  x <- 1
  r <- tryCatch({ class(x) <- k; attributes(x) }, error = function(e) conditionMessage(e))
  cat(k, ":", deparse(r), "\n")
}
f <- function() 1; class(f) <- "function"; print(attributes(f))
q <- quote(a); class(q) <- "name"; print(attributes(q))
e <- new.env(); class(e) <- "environment"; print(attributes(e))
s <- quote(a); class(s) <- "character"; print(s)
s <- quote(a); class(s) <- "list"; print(s)
s <- quote(a); class(s) <- "expression"; print(s)
x <- 1; try(class(x) <- "char")
try(`class<-`(y = 1, value = "a"))
e <- new.env(); try(class(e) <- "character"); print(class(e))
f <- sin; try(class(f) <- "integer")
s <- quote(a); try(class(s) <- "logical"); print(s)
x <- structure(1, class = "foo"); try(class(x) <- "name")
