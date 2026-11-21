# Exec/Tailcall chains far past options(expressions =) take no stack
# (eval.c's handle_exec_continuation loop); tailcall.R covers the same
# paths with short chains under the observation modes.
count <- function(n, acc) { force(acc); if (n == 0) acc else Tailcall(count, n - 1, acc + 1) }
print(count(200000, 0))
ev <- function(n) if (n == 0) TRUE else Exec(as.call(list(as.name("od"), n - 1)), globalenv())
od <- function(n) if (n == 0) FALSE else Tailcall(ev, n - 1)
print(ev(100001))
down <- function(n) if (n <= 0) "done" else Tailcall(sys.function(), n - 1)
print(down(50000))
run <- function() count(20000, 0)
for (k in 1:3) r <- run()
print(r)
