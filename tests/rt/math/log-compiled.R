# A compiled scalar log(v, b) that makes a NaN (GNU R's LOGBASE opcode)
# warns against its call; the interpreted one (math2) against the context
f <- function(v, b) log(v, b)
f(-1, 2)
f(2, -1)
f(2, -1)
