# Pure closures that assign locals, called from compiled code (robigo runs
# them inline, their locals in the inline run's slots): shadowing, a local
# assigned on one branch only (an unassigned one is looked up in the
# enclosure), a formal reassigned before it is forced (its argument is
# never evaluated), locals as arguments of nested calls, an error after an
# assignment, and observers in closures with locals. Each case runs three
# times in a compiled top-level loop: the first calls compile the
# closures, the last reaches the inline hook.
weigh <- function(x, y) { z <- x * y; z + 1 }
sw <- function(x) { x <- x * 2; x + 1 }
w <- 100
gl <- function(x) { w <- 5; w + x }
ul <- function(x) { if (x > 5) w <- 1; w + x }
rd <- function(x) { if (x > 0) z <- "pos" else z <- "neg"; z }
re <- function(x) { x <- 7; x }
add <- function(a, b) a + b
nest <- function(x) { t <- x + 1; u <- add(t, 2); add(u, t) }
chain <- function(n) { m <- n - 1; if (m < 1) 0 else chain(m) + m }
for (k in 1:3) r <- list(weigh(y = 2, x = k), sw(k), gl(k), ul(k), ul(10), rd(k), rd(-k),
                         re({cat("never\n"); 1}), nest(k), chain(5), w)
print(r)
late <- function(x) { z <- x * 2; z + "a" }
for (k in 1:3) r <- tryCatch(late(k), error = function(e) e)
print(conditionMessage(r)); print(conditionCall(r))
wl <- function(x) { z <- x + 1L; z * 2L }
for (k in 1:3) r <- withCallingHandlers(wl(.Machine$integer.max), warning = function(w) {
  if (k == 3) { print(conditionMessage(w)); print(conditionCall(w)) }
  invokeRestart("muffleWarning")
})
print(r)
sc <- function(x) { z <- x; sys.call() }
en <- function(x) { z <- x * 3; environment() }
mc <- function(alpha) { beta <- alpha; match.call() }
for (k in 1:3) r <- list(sc(k + 1), get("z", envir = en(k)), mc(k))
print(r)
