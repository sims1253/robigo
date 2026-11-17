# Byte-code translation cases: top-level code (compiler::compile) that
# eval() runs, and byte code as a value (scripts/bc-fixtures.R).
cases <- list(
  list(name = "t_arith", expr = quote(1 + 2 * 3), calls = c("print(eval(t_arith))", "print(typeof(t_arith))")),
  list(name = "t_block", expr = quote({ tv <- 3; tv * 2 }), calls = c("print(eval(t_block))", "print(tv)")),
  list(name = "t_loop", expr = quote({ s <- 0; for (i in 1:4) s <- s + i; s }), calls = "print(eval(t_loop))"),
  list(name = "t_env", expr = quote(a + b),
       calls = c("e <- new.env(); assign('a', 1, e); assign('b', 2, e)", "print(eval(t_env, e))",
                 "print(eval(t_env, list(a = 10, b = 20)))")),
  list(name = "t_twice", expr = quote(paste("n", n)),
       calls = c("n <- 1", "print(eval(t_twice))", "n <- 2", "print(eval(t_twice))")),
  list(name = "t_assign", expr = quote({ z <- 1:3; z[2] <- 0L; z }), calls = "print(eval(t_assign))")
)
