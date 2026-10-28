# Bits of GNU R's complex + - * / on special values (this R binary).
hex2d <- function(h) vapply(h, function(s) readBin(as.raw(strtoi(substring(s, seq(15, 1, -2), seq(16, 2, -2)), 16L)), "double"), 0, USE.NAMES = FALSE)
d2hex <- function(x) vapply(x, function(v) paste(rev(as.character(writeBin(v, raw()))), collapse = ""), "")
comp <- c("7ff00000000007a2", # NA_real_ (signalling)
          "7ff80000000007a2", # NA, quiet
          "fff8000000000000", # 0/0
          "7ff8000000000000", # -(0/0)
          "7ff8000000000123", # payload
          "fff4000000000456", # signalling payload, negative
          "7ff0000000000000", "fff0000000000000", # +-Inf
          "0000000000000000", "8000000000000000", # +-0
          "3ff0000000000000", "c008000000000000", # 1, -3
          "7fefffffffffffff", "0000000000000001") # max, min subnormal
v <- hex2d(comp)
z <- as.vector(outer(v, v, function(r, i) complex(real = r, imaginary = i)))
zh <- cbind(d2hex(Re(z)), d2hex(Im(z)))
n <- length(z)
ix <- rep(seq_len(n), each = n); iy <- rep(seq_len(n), n)
ops <- strsplit(Sys.getenv("OPS", "+ - * /"), " ")[[1]]
for (op in intersect(ops, c("+", "-", "*", "/"))) {
  r <- get(op)(z[ix], z[iy])
  writeLines(paste(c("add", "sub", "mul", "div")[match(op, c("+", "-", "*", "/"))],
                   zh[ix, 1], zh[ix, 2], zh[iy, 1], zh[iy, 2], d2hex(Re(r)), d2hex(Im(r))))
}

if ("%*%" %in% ops) {
  # n x 1 times 1 x n: element (i, k) is 0 + x_i * y_k
  m <- matrix(z, ncol = 1) %*% matrix(z, nrow = 1)
  writeLines(paste("matprod", zh[ix, 1], zh[ix, 2], zh[iy, 1], zh[iy, 2], d2hex(Re(m[cbind(ix, iy)])), d2hex(Im(m[cbind(ix, iy)]))))
}
if ("^" %in% ops) {
  r <- z[ix]^z[iy]
  writeLines(paste("pow", zh[ix, 1], zh[ix, 2], zh[iy, 1], zh[iy, 2], d2hex(Re(r)), d2hex(Im(r))))
}
