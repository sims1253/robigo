# radix-random.R at full volume, outside gctorture (radix-volume.requires:
# volume): order() by "radix" and by "shell" over 7,200 small generated
# vectors (a multiplicative hash of the index: ties, NA, NaN, -0 and
# negative values), one to three keys, every na.last and decreasing, each
# configuration summed into a checksum of the orders it gives; then 600 and
# 70,000 rows that reach each of forder's passes, in both directions.
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
sweep <- function(ncases) for (method in c("radix", "shell")) for (nalast in list(TRUE, FALSE, NA)) for (k in 1:3) {
  acc <- 0
  for (case in 1:ncases) {
    n <- 1 + h(case) %% 12
    keys <- lapply(seq_len(k), function(j) gen(case * 3 + j, n, types[1 + (case + j) %% 4]))
    dec <- if (method == "radix") (h(case + 7) %% 2^seq_len(k)) >= 2^(seq_len(k) - 1) else h(case) %% 2 == 1
    o <- do.call(order, c(keys, list(na.last = nalast, decreasing = dec, method = method)))
    acc <- acc + sum_o(o) * (1 + case %% 5)
  }
  cat(method, as.character(nalast), k, acc, "\n")
}
# n rows past forder's insertion sort (200): integers by counting (a range
# within the rows and 100,000, in each group too) and by bytes (a range past
# 2^16), logicals, doubles with
# every special value, strings of several lengths that share prefixes; and
# a first key whose groups fall on both sides of 200 rows
big <- function(n) {
  v <- h(1:n)
  na <- v %% 13 == 0
  count <- as.integer(v %% 100) - 50L
  bytes <- as.integer(v %% 2^30) - 536870912L
  lgl <- v %% 3 == 0
  dbl <- (v %% 1e6 - 5e5) / 7
  chr <- paste0(c("", "a", "ab", "abc", "b", "ba")[1 + v %% 6], c("", "x", "xyz")[1 + (v %/% 7) %% 3], (v %/% 21) %% 40)
  count[na] <- NA
  bytes[na] <- NA
  lgl[na] <- NA
  dbl[na] <- NA
  dbl[v %% 17 == 0] <- NaN
  dbl[v %% 19 == 0] <- -0
  dbl[v %% 23 == 0] <- 0
  dbl[v %% 29 == 0] <- Inf
  dbl[v %% 31 == 0] <- -Inf
  chr[na] <- NA
  list(keys = list(count = count, bytes = bytes, lgl = lgl, dbl = dbl, chr = chr),
       groups = as.integer(v %% 1000 < 400) + as.integer(v %% 1000 < 650))
}
# each key alone and after the groups, by radix, for each na.last and the
# directions in `dirs` (the groups' direction, the key's the opposite)
big_orders <- function(b, dirs) {
  g <- b$groups
  print(c(sum(g == 0L), sum(g == 1L), sum(g == 2L)))
  nalasts <- list(TRUE, FALSE, NA)
  for (nm in names(b$keys)) for (i in 1:3) {
    x <- b$keys[[nm]]
    ds <- dirs(i)
    one <- vapply(ds, function(d) sum_o(order(x, na.last = nalasts[[i]], decreasing = d, method = "radix")), 0)
    two <- vapply(ds, function(d) sum_o(order(g, x, na.last = nalasts[[i]], decreasing = c(d, !d), method = "radix")), 0)
    cat(nm, as.character(nalasts[[i]]), one, two, "\n")
  }
}
sweep(400)
big_orders(big(600), function(i) c(FALSE, TRUE))
big_orders(big(70000), function(i) c(FALSE, TRUE))
