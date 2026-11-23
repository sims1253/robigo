# Comparing raw with numbers (relop.c coerces the raw operand)
as.raw(2) == 2
as.raw(c(1, 255)) < 3L
as.raw(2) == TRUE
as.raw(0) == FALSE
2.5 > as.raw(2)
as.raw(1:3) == as.raw(c(1, 0, 3))
as.raw(3) == "03"
