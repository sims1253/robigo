# R: eval.c:applydefine re-evaluates an outer `[[<-`'s index before that
# setter stores the inner setter's value. A value the inner setter
# allocated anew (here `[<-` coerces to double) is not yet in the target;
# nor is an in-place change to an element that R code in the getter's
# index referenced (`print(a$b)` leaves its reference count raised, so
# eval.c:evalseq copies it). Both times the index sees the old element.
f <- function() {
  a <- list(b = 1:3)
  a[[{ print(a$b); "b" }]][2] <- 10.5
  print(a)
  a <- list(b = 1:3)
  a[[{ print(a$b); "b" }]][2] <- 0L
  print(a)
}
f()
