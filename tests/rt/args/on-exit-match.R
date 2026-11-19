# on.exit()'s own argument matching (builtin.c do_onexit, matchArgs_NR):
# its errors against the call, unused arguments deparsed.
show <- function(f) {
  r <- tryCatch(f(), error = function(e) e)
  print(conditionMessage(r))
  print(conditionCall(r))
  invisible()
}
show(function() on.exit(1, TRUE, FALSE, 4))
show(function() on.exit(1, foo = 2))
show(function() on.exit(a = 1, a = 2))
show(function() on.exit(1, add = TRUE, add = FALSE))
f <- function() { on.exit(cat("bye\n"), after = FALSE, add = TRUE); "ok" }
f()
