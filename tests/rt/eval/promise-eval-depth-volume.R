# R: eval.c:eval (PROMSXP). C code that evaluates a promise with
# eval(x, rho) takes eval's depth step before forcePromise: the promise's
# code runs one level deeper than a bare force. Each line forces a fresh
# promise whose code measures the levels left below the limit.
N <- 0L
g <- compiler::cmpfun(function() { N <<- N + 1L; g() })
hr <- compiler::cmpfun(function() { N <<- 0L; tryCatch(g(), error = function(e) NULL); N })
e <- environment()
mk <- function() delayedAssign("p", hr(), assign.env = e)
options(expressions = 200L)
cat("direct:            ", hr(), "\n")
mk(); cat("symbol:            ", p, "\n")
mk(); cat("e$p:               ", e$p, "\n")
mk(); cat("e[[\"p\"]]:          ", e[["p"]], "\n")
mk(); cat("get(\"p\"):          ", get("p"), "\n")
mk(); cat("get0(\"p\"):         ", get0("p"), "\n")
mk(); cat("mget(\"p\"):         ", mget("p", envir = e)[[1L]], "\n")
f <- new.env()
mkf <- function() delayedAssign("q", hr(), assign.env = f)
mkf(); cat("as.list(f):        ", as.list(f)$q, "\n")
mkf(); cat("eapply(f, I):      ", eapply(f, function(v) v)$q, "\n")
gen <- function(x) UseMethod("gen")
gen.default <- function(x) x
cat("UseMethod (fresh): ", gen(hr()), "\n")
gen2 <- function(x) { x; UseMethod("gen") }
cat("UseMethod (forced):", gen2(hr()), "\n")
options(expressions = 5000L)

# a promise interrupted at the limit by UseMethod's GetObject restarts once
k <- compiler::cmpfun(function(x, d) if (d > 0L) k(x, d - 1L) else { last <<- environment(); UseMethod("k") })
k.default <- function(x, d) x
last <- NULL
options(expressions = 25L)
r1 <- tryCatch(k(42L + 0L, 13L), error = function(x) conditionMessage(x))
options(expressions = 5000L)
W <- 0L
r2 <- tryCatch(withCallingHandlers(last$x, warning = function(w) {
    W <<- W + 1L
    invokeRestart("muffleWarning")
}), error = function(x) conditionMessage(x))
print(r1)
print(r2)
cat("restart warnings:", W, "\n")
