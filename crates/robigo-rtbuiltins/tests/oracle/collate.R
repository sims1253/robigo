## Scollate under ICU's root collator (C.UTF-8's) in min, max, pmin and
## pmax of strings, with every encoding mark (the orderings are relop.R's)
s <- list(a = c("b", "A", "a", "B"), e = c("e", "\u00e9", "E", "f", "e\u0301"), n = c("b", NA, "a"),
          sc = c("\u4e00", "\U000218a2", "\u03b1", "\uac00", "z"), emp = c("", "a"),
          l1 = iconv(c("\u00e9", "z"), "UTF-8", "latin1"), u = c("\u00e9", "e"),
          nat = `Encoding<-`("\u00e9", "unknown"), bad = c("\xff", "\ufffd"),
          by = `Encoding<-`(c("\xe9", "a"), "bytes"), num = c(10, 9), int = c(2L, 10L), none = character())
for (f in c("min", "max")) for (i in names(s)) {
  case(paste0(f, ".", i), call(f, quote(x)), list(x = s[[i]]))
  for (j in c("a", "n", "l1", "num", "by"))
    case(paste0(f, ".", i, ".", j), call(f, quote(x), quote(y)), list(x = s[[i]], y = s[[j]]))
}
for (f in c("pmin", "pmax")) for (r in c(FALSE, TRUE)) for (i in names(s)) for (j in c("a", "n", "l1", "u", "by"))
  case(paste0(f, ".", r, ".", i, ".", j),
       if (f == "pmin") quote(.Internal(pmin(r, x, y))) else quote(.Internal(pmax(r, x, y))),
       list(r = r, x = s[[i]], y = s[[j]]), internal = TRUE)
## not FCD to ICU's collation (robigo: not implemented yet)
case("max.nonfcd", quote(max(x)), list(x = c("a\u0301\u0323", "a\u0323\u0301")))
case("pmin.nonfcd", quote(.Internal(pmin(r, x, y))), list(r = FALSE, x = "a\u0301\u0323", y = "a\u0323\u0301"),
     internal = TRUE)
