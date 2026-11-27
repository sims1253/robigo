# format(), formatC(), and sprintf() long enough to collect inside one call
# (Cx::fill_strings) under gctorture, young-moving GC, and retry-torture:
# each pass re-reads its inputs from roots and reuses its formatting buffers
# without leaking partial state across retries or moves.

# format() across all atomic types + attributes + decimal.mark warnings
r <- format((1:800) / 7)
cat("fmt-real:", length(r), r[1], r[400], r[800], "\n")

iv <- 1:500
names(iv) <- paste0("k", 1:500)
fi <- format(iv)
cat("fmt-int:", length(fi), fi[1], fi[500], names(fi)[1], names(fi)[500], "\n")

lv <- rep(c(TRUE, FALSE, NA), length.out = 400)
fl <- format(lv)
cat("fmt-lgl:", length(fl), fl[1], fl[2], fl[3], fl[400], "\n")

cv <- complex(real = seq(-10, 10, length.out = 300), imaginary = 1:300)
cv[150] <- NA
fc <- format(cv, digits = 4)
cat("fmt-cplx:", length(fc), fc[1], fc[150], fc[300], "\n")

sv <- rep(c("ab", "été", "中文", NA_character_), length.out = 500)
fs1 <- format(sv, justify = "centre")
fs2 <- format(sv, na.encode = FALSE)
cat("fmt-str:", length(fs1), fs1[1], fs1[2], fs1[3], fs1[4], is.na(fs2[4]), "\n")

m <- matrix(1:200, nrow = 10, dimnames = list(paste0("r", 1:10), paste0("c", 1:20)))
fm <- format(m)
cat("fmt-mat:", dim(fm), fm[1, 1], fm[10, 20], rownames(fm)[10], colnames(fm)[20], "\n")

wcount <- 0L
withCallingHandlers(
  fw <- format(c(1.5, 2.5, 3.5), decimal.mark = "::"),
  warning = function(w) { wcount <<- wcount + 1L; invokeRestart("muffleWarning") }
)
cat("fmt-warn:", wcount, fw, "\n")

# formatC() integer, double f, double fg (0, <1e-4, >=1e-4), and length-0
cd <- formatC(1:500, format = "d", width = 5, flag = "0")
cf <- formatC((1:500) / 3, format = "f", digits = 3)
cfg <- formatC(rep(c(0, 1.23e-6, 3.14159, 123456), length.out = 400), format = "fg", digits = 3)
cat("fmtC:", length(cd), cd[1], cd[500], cf[1], cf[500], cfg[1], cfg[2], cfg[3], cfg[4], "\n")
cat("fmtC-zero:", length(formatC(integer(0), format = "d")), length(formatC(numeric(0), format = "f")), "\n")

# sprintf() single format, recycled multi-format, UTF-8, *, positional, row-0 R coercion
sp1 <- sprintf("s%04d", 1:800)
cat("sp-int:", length(sp1), sp1[1], sp1[800], "\n")

sp2 <- sprintf("%3$s:%*d", rep(c(3L, 5L), 250), 1:500, rep(c("é", "a"), 250))
cat("sp-mix:", length(sp2), sp2[1], sp2[2], sp2[500], Encoding(sp2[1]), Encoding(sp2[2]), "\n")

sp3 <- sprintf(c("a=%d", "b=%04x"), 1:400)
cat("sp-multi:", length(sp3), sp3[1], sp3[2], sp3[399], sp3[400], "\n")

# Row-0 S3 as.character() coercion + exact double-to-integer coercion
obj <- structure(1:400, class = "fmtc_s3")
as.character.fmtc_s3 <- function(x, ...) paste0("v", unclass(x))
sp4 <- sprintf("c=%s", obj)
sp5 <- sprintf("i=%d", as.double(1:400))
cat("sp-coerce:", length(sp4), sp4[1], sp4[400], sp5[1], sp5[400], "\n")

# Truncation warning on row 0 and row >= 1 + warning-before-error on row >= 1
big <- paste(rep("x", 9000), collapse = "")
tw <- character(0)
withCallingHandlers(
  sp6 <- sprintf("%.4s", c(big, "ok", big)),
  warning = function(w) { tw <<- c(tw, conditionMessage(w)); invokeRestart("muffleWarning") }
)
cat("sp-trunc:", length(tw), sp6, "\n")

tw2 <- character(0)
err2 <- tryCatch(
  withCallingHandlers(
    sprintf(c("%.4s", "%.4s", "%d"), c("ok", big, "bad")),
    warning = function(w) { tw2 <<- c(tw2, conditionMessage(w)); invokeRestart("muffleWarning") }
  ),
  error = conditionMessage
)
cat("sp-warn-err:", length(tw2), err2, "\n")

# Unused argument warning
uw <- character(0)
withCallingHandlers(
  sp7 <- sprintf("x=%d", 1:10, 99L),
  warning = function(w) { uw <<- c(uw, conditionMessage(w)); invokeRestart("muffleWarning") }
)
cat("sp-unused:", length(sp7), sp7[10], uw, "\n")
