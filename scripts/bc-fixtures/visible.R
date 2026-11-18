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
       calls = c("k_retinvisible(TRUE)", "k_retinvisible(FALSE)")),
  # a tail `&&` sets visibility on its own paths (AND1ST and AND2ND), not
  # at a RETURN another branch shares
  list(name = "k_retand", fun = function(c, x, y) return(if (c) x && y else invisible(1)),
       calls = c("k_retand(TRUE, TRUE, FALSE)", "k_retand(TRUE, FALSE, TRUE)", "k_retand(FALSE, TRUE, TRUE)")),
  list(name = "k_retand_last", fun = function(c, x, y) return(if (c) invisible(1) else x || y),
       calls = c("k_retand_last(TRUE, TRUE, TRUE)", "k_retand_last(FALSE, FALSE, TRUE)",
                 "k_retand_last(FALSE, TRUE, FALSE)")),
  list(name = "k_tailand", fun = function(x, y) { invisible(x); x && y },
       calls = c("k_tailand(TRUE, TRUE)", "k_tailand(FALSE, TRUE)")),
  # an inner `&&` whose AND2ND no path reaches (its second operand
  # returns): the outer `&&` is paired with its own AND1ST, by its join
  list(name = "k_and_inner_return", fun = function(a, b) a && { b && return(TRUE); FALSE },
       calls = c("k_and_inner_return(TRUE, FALSE)", "k_and_inner_return(TRUE, TRUE)",
                 "k_and_inner_return(FALSE, TRUE)")),
  # a tail `&&` or `||` whose AND2ND no path reaches: AND1ST's path is
  # visible, after an assignment left it invisible
  list(name = "k_and_return", fun = function() { a <- FALSE; a && return(TRUE) }, calls = "k_and_return()"),
  list(name = "k_or_return", fun = function() { a <- TRUE; a || return(FALSE) }, calls = "k_or_return()")
)
