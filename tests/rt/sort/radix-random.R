# order() by "radix" and by "shell" over many small generated vectors (a
# multiplicative hash of the index: ties, NA, NaN, -0 and negative values),
# one to three keys, every na.last and decreasing; each configuration
# summed into a checksum of the orders it gives
h <- function(i) ((i * 2654435761) %% 4294967296)
gen <- function(seed, n, type) {
  v <- h(seed * 1000 + seq_len(n))
  x <- switch(type,
              int = as.integer(v %% 7) - 3L,
              dbl = (v %% 11 - 5) / 2,
              lgl = (v %% 3) == 0,
              chr = c("b", "a", "B", "", "ab", "A")[1 + v %% 6])
  x[v %% 13 == 0] <- NA
  if (type == "dbl") {
    x[v %% 17 == 0] <- NaN
    x[v %% 19 == 0] <- -0
  }
  x
}
sum_o <- function(o) sum(as.numeric(o) * seq_along(o)) + 1000 * length(o)
types <- c("int", "dbl", "lgl", "chr")
for (method in c("radix", "shell")) for (nalast in list(TRUE, FALSE, NA)) for (k in 1:3) {
  acc <- 0
  for (case in 1:400) {
    n <- 1 + h(case) %% 12
    keys <- lapply(seq_len(k), function(j) gen(case * 3 + j, n, types[1 + (case + j) %% 4]))
    dec <- if (method == "radix") (h(case + 7) %% 2^seq_len(k)) >= 2^(seq_len(k) - 1) else h(case) %% 2 == 1
    o <- do.call(order, c(keys, list(na.last = nalast, decreasing = dec, method = method)))
    acc <- acc + sum_o(o) * (1 + case %% 5)
  }
  cat(method, as.character(nalast), k, acc, "\n")
}
