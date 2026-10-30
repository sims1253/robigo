# Default output retains R_CurrentExpression even without an eval wrapper.
{ x <- 1; x[[]] <- 42; x }
