# Conditions raised in C reach R's calling handlers through base's hooks
# (errors.c): a simple error as .handleSimpleError(h, msg, base::quote(call)),
# a warning through .signalSimpleWarning(msg, base::quote(call)) and its
# withRestarts(muffleWarning = ). A classed C error goes to h(cond) directly.
heads <- function() {
  calls <- sys.calls()
  h <- vapply(calls, function(x) paste(deparse(x[[1]]), collapse = ""), "")
  h <- h[!startsWith(h, "function")]
  print(h[max(1, length(h) - 5):length(h)])
}
f <- function() log("a")
r <- tryCatch(withCallingHandlers(f(), error = function(e) {
  heads(); print(class(e)); print(conditionCall(e))
}), error = function(e) conditionMessage(e))
r
g <- function() stop("boom")
r <- tryCatch(withCallingHandlers(g(), error = function(e) heads()), error = function(e) conditionMessage(e))
r
k <- function() as.integer("x")
withCallingHandlers(k(), warning = function(w) {
  heads(); print(class(w)); print(conditionCall(w))
  invokeRestart("muffleWarning")
})
w <- function() warning("careful")
withCallingHandlers(w(), warning = function(w) { heads(); invokeRestart("muffleWarning") })
withCallingHandlers(invisible(as.integer("y")), warning = function(w) invokeRestart("muffleWarning"))
m <- function(a) a
r <- tryCatch(withCallingHandlers(m(), error = function(e) { heads(); print(class(e)) }),
              error = function(e) conditionMessage(e))
r
