# cbind/rbind deparse.level = 2 tags: bind.c uses deparse1line(expr, TRUE),
# which keeps the first 10 bytes of the deparsed first line and appends "..."
# when it was longer (deparse.c:deparse1WithCutoff's abbrev arm).
x <- 1:2
print(colnames(cbind(x, x + 1, x * 10, deparse.level = 2)), quote = FALSE)
print(colnames(cbind(x, sinpi(x), cospi(x), 1234567890123, deparse.level = 2)), quote = FALSE)
print(rownames(rbind(a = x, x + 1, 1:2 + 1, deparse.level = 2)), quote = FALSE)
# a symbol tag (deparse.level = 1) and an explicit tag are untouched
print(colnames(cbind(y = x, x, deparse.level = 1)), quote = FALSE)
