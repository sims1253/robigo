# Byte-code translation cases: a value that reaches RETURN through GOTOs
# is in tail position, visible as bcEval leaves it (its loads set
# R_Visible), though cmp.R compiles `return(x)`'s operand outside the tail,
# so each branch of `return(if ...)` but the last jumps to the RETURN;
# after an invisible() call, which would leave the value invisible
# (scripts/bc-fixtures.R).
cases <- list(
  list(name = "k_retif", fun = function(x) { invisible(x); return(if (x) NULL else 2) },
       calls = c("k_retif(TRUE)", "k_retif(FALSE)")),
  list(name = "k_retif_var", fun = function(x, y) { invisible(x); return(if (x) y else 2) },
       calls = c("k_retif_var(TRUE, 5)", "k_retif_var(FALSE, 5)")),
  list(name = "k_retif_const", fun = function(x) { invisible(x); return(if (x) "a" else "b") },
       calls = c("k_retif_const(TRUE)", "k_retif_const(FALSE)")),
  list(name = "k_retif_nested", fun = function(x, y) { invisible(x); return(if (x) (if (y) 1 else 2) else 3) },
       calls = c("k_retif_nested(TRUE, TRUE)", "k_retif_nested(TRUE, FALSE)", "k_retif_nested(FALSE, TRUE)")),
  list(name = "k_tailif", fun = function(x) { invisible(x); if (x) 1 else 2 },
       calls = c("k_tailif(TRUE)", "k_tailif(FALSE)")),
  list(name = "k_retinvisible", fun = function(x) { invisible(x); return(if (x) invisible(1) else 2) },
       calls = c("k_retinvisible(TRUE)", "k_retinvisible(FALSE)"))
)
