# return()'s operand, its visibility: the AST's at a closure's first call,
# compiled code's after the JIT compiles it. cmp.R compiles the operand
# outside the tail (make.nonTailCallContext), so it is as visible as bcEval
# leaves it at RETURN: a missing else, a switch's miss and a loop give a
# visible NULL (LDNULL), an assignment its value's visibility (SETVAR and
# ENDASSIGN do not touch R_Visible), invisible() stays invisible. The AST
# returns them as do_set and the specials leave them. (A closure with a loop
# is compiled at its first call.) Each closure has a body of its own (the
# JIT cache).
f01 <- function(c) return(if (c) 1)
f02 <- function(k) return(switch(k, "a", "b"))
f03 <- function(v) return(x <- v)
f04 <- function(n) return(for (i in seq_len(n)) i)
f05 <- function(c) return(while (c) break)
f06 <- function(r) return(repeat break)
f07 <- function(v) return(invisible(v))
f08 <- function(v) return({ y <- v })
f09 <- function(v) return(z <- w <- v)
f10 <- function(c, v) if (c) return(q <- v) else 2
f11 <- function(v) return(if (v > 0) u <- v else -v)
f12 <- function(k) return(switch(k, a = , b = "ab", "other"))
f13 <- function(v) { s <- v; return(s <<- v) }
f14 <- function(v) return((p <- v))
cat("first (AST):\n")
cat("f01: "); f01(FALSE); cat("\n")
cat("f02: "); f02(3); cat("\n")
cat("f03: "); f03(1); cat("\n")
cat("f04: "); f04(2); cat("\n")
cat("f05: "); f05(TRUE); cat("\n")
cat("f06: "); f06(1); cat("\n")
cat("f07: "); f07(1); cat("\n")
cat("f08: "); f08(1); cat("\n")
cat("f09: "); f09(1); cat("\n")
cat("f10: "); f10(TRUE, 1); cat("\n")
cat("f11: "); f11(1); cat("\n")
cat("f12: "); f12("z"); cat("\n")
cat("f13: "); f13(1); cat("\n")
cat("f14: "); f14(1); cat("\n")
for (i in 1:3) { f01(FALSE); f02(3); f03(1); f04(2); f05(TRUE); f06(1); f07(1); f08(1); f09(1); f10(TRUE, 1); f11(1); f12("z"); f13(1); f14(1) }
cat("compiled:\n")
cat("f01: "); f01(FALSE); cat("\n")
cat("f02: "); f02(3); cat("\n")
cat("f03: "); f03(1); cat("\n")
cat("f04: "); f04(2); cat("\n")
cat("f05: "); f05(TRUE); cat("\n")
cat("f06: "); f06(1); cat("\n")
cat("f07: "); f07(1); cat("\n")
cat("f08: "); f08(1); cat("\n")
cat("f09: "); f09(1); cat("\n")
cat("f10: "); f10(TRUE, 1); cat("\n")
cat("f11: "); f11(1); cat("\n")
cat("f12: "); f12("z"); cat("\n")
cat("f13: "); f13(1); cat("\n")
cat("f14: "); f14(1); cat("\n")

# complex and super assignments, and an invisible value
g15 <- function(v) { x <- 1:2; return(names(x) <- v) }
g16 <- function(v) { x <- 1:2; return(x[1] <- v) }
g17 <- function(v) return(x <- invisible(v))
g18 <- function(v) { x <- 1; return(attr(x, "a") <- v) }
g19 <- function(v) { x <- list(); return(x$a <- v) }
`h<-` <- function(x, value) { x }
g20 <- function(v) { x <- 1; return(h(x) <- v) }
g21 <- function(v) return(y <- v)
# no step of `x$a <- 2` sets R_Visible, so the value's load does; an
# index's load sets it after an invisible value
g22 <- function(v) { x <- list(a = 1); invisible(v); return(x$a <- v) }
g23 <- function(v) { x <- 1:2; return(x[1] <- invisible(v)) }
for (i in 1:4) { g15("a"); g16(9); g17(1); g18(2); g19(3); g20(4); g21(5); g22(6); g23(7L) }
cat("g15: "); g15("a"); cat("\n")
cat("g16: "); g16(9); cat("\n")
cat("g17: "); g17(1); cat("\n")
cat("g18: "); g18(2); cat("\n")
cat("g19: "); g19(3); cat("\n")
cat("g20: "); g20(4); cat("\n")
cat("g21: "); g21(5); cat("\n")
cat("g22: "); g22(6); cat("\n")
cat("g23: "); g23(7L); cat("\n")
