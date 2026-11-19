# Recall at top level: error() reports Recall's own call.
f <- function(n) if (n) Recall(n - 1) else "ok"
f(1)
Recall(1)
