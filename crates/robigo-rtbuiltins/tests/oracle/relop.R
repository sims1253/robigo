## relop.c: == != < <= >= > (do_relop, do_relop_dflt)
nums <- list(NULL0 = NULL, i1 = 5L, i0 = integer(0), iNA = NA_integer_, i3 = c(-7L, 2L, NA), d1 = 2,
             dNA = NA_real_, dNaN = NaN, d3 = c(-7, 2.5, Inf), l1 = TRUE, l3 = c(TRUE, FALSE, NA),
             r2 = as.raw(c(1, 255)), r1 = as.raw(2))
for (op in c("==", "!=", "<", ">=")) binary(paste0("n", op), op, nums)
binary("n<=", "<=", nums[c("i3", "d3", "l3", "r2")])
binary("n>", ">", nums[c("i3", "d3", "l3", "r2")])
## strings: == and != in C.UTF-8 (Seql), mixed with numbers (coerced)
strs <- list(a = c("a", "b", NA), one = "a", num = c("1", "2.5", "1e+05", "TRUE"), d = c(1, 2.5, 1e5, 0.1 + 0.2),
             i = c(1L, NA), l = c(TRUE, NA), e = character(0), sym = quote(a), r = as.raw(c(1, 16)),
             u = "é", l1 = iconv("é", "UTF-8", "latin1"), nat = `Encoding<-`("é", "unknown"),
             by = `Encoding<-`("\xe9", "bytes"), by2 = `Encoding<-`("\xe9", "bytes"))
for (op in c("==", "!=")) binary(paste0("s", op), op, strs)
## complex
cx <- list(z = c(1+1i, NA, 2), d = 2, i = NA_integer_, s = "1+1i")
binary("z==", "==", cx[1:3])
binary("z!=", "!=", cx[1:3])
case("z<", quote(x < y), list(x = 1i, y = 1))
case("z<0", quote(x < y), list(x = complex(0), y = 1))
## attributes
at <- list(nm = c(a = 1, b = 2), nm3 = c(x = 1L, y = 2L, z = 3L), mat = matrix(1:4, 2, dimnames = list(c("r1", "r2"), NULL)),
           mat2 = matrix(c(1, 2, 3, 4), 2), plain2 = c(10, 20), ts3 = structure(c(1, 2, 3), tsp = c(1, 3, 1), class = "ts"),
           foo = structure(1:2, foo = "bar", class = "foo"), e0 = numeric(0))
binary("a<", "<", at)
case("a.nonconf", quote(x == y), list(x = matrix(1:4, 2), y = matrix(1:6, 2)))
case("a.tsshort", quote(x == y), list(x = structure(1:2, tsp = c(1, 2, 1)), y = 1:3))
## language
case("lang.sym", quote(x == y), list(x = quote(a), y = quote(a)))
case("lang.sym2", quote(x != y), list(x = quote(a), y = quote(b)))
case("lang.lt", quote(x < y), list(x = quote(a), y = "b"))
case("bad.fun", quote(x == y), list(x = baseenv(), y = 1))
case("bad.argc", quote(`==`(x)), list(x = 1))
case("bad.list", quote(x == y), list(x = 1, y = list(1)))
## recycling
case("recyc", quote(x == y), list(x = 1:3, y = 1:2))
case("recyc.str", quote(x == y), list(x = c("a", "b", "c"), y = c("a", "b")))
## orderings under ICU's root collator (C.UTF-8's): case, accents, digits,
## punctuation, empty strings, NA, other scripts, and every encoding mark
icu <- list(s = c("a", "B", "b", "A", "_", "", NA, "10", "9", "a b", "a-b", "ab"),
            acc = c("e", "\u00e9", "E", "\u00c9", "e\u0301", "f", "\u00e8"),
            sc = c("\u03b1", "\u0430", "\u05d0", "\u0627", "\u4e00", "\U000218a2", "\uac00", "\u3042", "\U0001F600", "\ufffd"),
            n = c(10, 9, -1.5), l1 = iconv(c("\u00e9", "z", "E"), "UTF-8", "latin1"),
            nat = `Encoding<-`("\u00e9", "unknown"), bad = c("\xff", "a\xe9"),
            by = `Encoding<-`("\xe9", "bytes"))
for (op in c("<", ">", "<=", ">=")) binary(paste0("I.", op), op, icu)
## text that is not FCD to ICU's collation (robigo: not implemented yet)
case("I.nonfcd", quote(x < y), list(x = "a\u0301\u0323", y = "a\u0323\u0301"))
case("I.tibetan", quote(x < y), list(x = "\u0f71\u0f81", y = "\u0f71\u0f71\u0f80"))
## orderings under C collation (Sys.setlocale), on strings and coerced numbers
invisible(Sys.setlocale("LC_COLLATE", "C"))
ord <- list(s = c("a", "B", "_", "a", NA, "10", "9"), t = "a", n = c(10, 9), u = c("é", "z"), l1 = iconv("é", "UTF-8", "latin1"),
            by = `Encoding<-`("\xe9", "bytes"))
for (op in c("<", ">", "<=", ">=")) binary(paste0("C.", op), op, ord)
