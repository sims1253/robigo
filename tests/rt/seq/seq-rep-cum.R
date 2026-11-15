# What the E7 rows reach through seq() (benchmarks/rt/calls-rows.toml,
# group e7): seq.default's integer `by` path, with `.Machine`
# (machine.c's Init_R_Machine), .Internal(rep.int()) (seq.c's do_rep_int,
# rep2, rep3) and cumsum (cum.c's do_cum, with cumprod, cummax and cummin),
# on GNU R 4.6.2.
keys <- paste0("k", 1:1000)
q <- keys[seq(1L, 1000L, by = 10L)]
print(length(q)); print(q[1:3]); print(q[100])
print(match(q, keys)[1:5])
dup <- rep(keys[1:100], 10L)
print(length(unique(dup)))
print(paste(q, 7L)[1:2])
print(seq(2L, 11L, by = 3L)); print(seq(1, 2, by = 0.25)); print(seq(5L, 1L, by = -2L))
print(rep.int(1:3, 2:0)); print(rep.int(c(a = 1, b = 2), 2)); print(rep.int(list(1, "a"), 2)); print(rep.int(1, 2.9)); print(rep.int("a", TRUE))
f <- rep.int(structure(1:2, levels = c("x", "y"), class = "factor"), 2); print(unclass(f))
err <- function(e) conditionMessage(e)
print(tryCatch(rep.int(NULL, 3), error = err)); print(tryCatch(rep.int(NULL, 1L[0]), error = err))
print(tryCatch(rep.int(1, NA), error = err)); print(tryCatch(rep.int(1, -1), error = err))
print(tryCatch(rep.int(1:2, c(1, NA)), error = err)); print(tryCatch(rep.int(1, sum), error = err))
print(tryCatch(rep.int(sum, 2), error = err)); print(tryCatch(rep.int(1:2, 1:3), error = err))
print(tryCatch(rep.int(1:2, as.raw(2)), error = err)); print(tryCatch(rep.int(1:2, list(2)), error = err))
print(rep.int(1, as.raw(2)))
print(cumsum(1:10)); print(cumsum(c(a = 1.5, b = 2, c = NA, d = 1))); print(cumsum(c(1, NaN, NA, 2)))
print(cumsum(c(TRUE, FALSE, TRUE))); print(cumsum(c(.Machine$integer.max, 1L)))
print(cumprod(1:6)); print(cumprod(c(2, NA, 3))); print(cummax(c(1, 3, 2, NaN, 5))); print(cummin(c(5L, 3L, NA, 1L)))
print(cummax(c(NA, 1L))); print(cummin(c(NA, 1L))); print(cumsum(0[0])); print(cumsum("3")); print(cumsum(c(0.1, 0.2, 0.3)) == c(0.1, 0.3, 0.6))
print(.Machine$double.eps); print(.Machine$integer.max); print(names(.Machine)[1:3]); print(length(.Machine))
# every entry, exactly
for (n in names(.Machine)) cat(n, typeof(.Machine[[n]]), sprintf("%.17g", .Machine[[n]]), "\n")
