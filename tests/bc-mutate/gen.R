# Hostile byte code for robigo-rt (#976): mutated code vectors of
# compiler::cmpfun'd closures, made into closures with .Internal(mkCode) and
# .Internal(bcClose) and called. scripts/rt-bc-mutate.py runs it; its
# regression corpus (corpus.txt) names mutations by seed and number, so a
# change here changes what each entry means (the harness pins this file's
# hash).
#
# BCM_SEED: the generator's seed; BCM_BATCH: mutations 1..BATCH, each
# announced on stderr ("mutation K") before it runs; BCM_ONLY: run mutation
# BCM_ONLY alone (the ones before it drawn, not run).
seed <- as.integer(Sys.getenv("BCM_SEED")); batch <- as.integer(Sys.getenv("BCM_BATCH"))
only <- as.integer(Sys.getenv("BCM_ONLY", "0"))
# an LCG in plain arithmetic (robigo-rt has no .Internal(sample)): 1..n
st <- seed
rn <- function(n) { st <<- (st * 69069 + 1) %% 4294967296; 1L + as.integer(floor(st / 4294967296 * n)) }
pick <- function(v) v[rn(length(v))]
fs <- list(
  compiler::cmpfun(function(x) { s <- 0; for (i in seq_len(x)) s <- s + i; s }),
  compiler::cmpfun(function(v, i) { v[i] <- v[i] * 2; v[[1]] + v[i] }),
  compiler::cmpfun(function(l) { l$a <- 1; l[["b"]] <- l$a + 1; names(l) }),
  compiler::cmpfun(function(x) switch(x, a = 1, b = { y <- 2; y + 1 }, 3)),
  compiler::cmpfun(function(n) { k <- 0; while (k < n) { k <- k + 1; if (k > 3) break }; k }),
  compiler::cmpfun(function(m) { for (r in seq_len(nrow(m))) m[r, 1] <- r; m }),
  compiler::cmpfun(function(x, ...) { z <- list(...); for (e in z) x <- x + e; x }),
  compiler::cmpfun(function(a) { f <- function(b) a + b; f(2) * 3 })
)
args <- list(list(4), list(c(1, 2, 3), 2), list(list(a = 0)), list("b"), list(6), list(matrix(0, 3, 2)),
             list(1, 2, 3), list(5))
for (k in seq_len(batch)) {
  j <- rn(length(fs))
  f <- fs[[j]]
  d <- .Internal(disassemble(.Internal(bodyCode(f))))
  code <- as.integer(d[[2]]); consts <- d[[3]]
  n <- length(code)
  for (m in seq_len(rn(3))) {
    p <- 1L + rn(n - 1L)
    code[p] <- switch(rn(3),
      pick(0:(length(consts) + 3L)),
      pick(0:130),
      code[1L + rn(n - 1L)])
  }
  if (only > 0L && k != only) next
  message("mutation ", k)
  r <- tryCatch({
    bc <- .Internal(mkCode(code, consts))
    g <- .Internal(bcClose(formals(f), bc, environment(f)))
    do.call(g, args[[j]])
    "ran"
  }, error = function(e) "error", warning = function(w) "warning")
}
cat("done\n")
