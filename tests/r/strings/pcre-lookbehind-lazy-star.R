# PCRE rejects unbounded lookbehind, and a lazy group under a star keeps looping.
err <- function(p) tryCatch(suppressWarnings(grepl(p, "aab", perl = TRUE)),
                            error = function(e) conditionMessage(e))
print(err("(?<=a+)b"))
print(err("(?<!a*)b"))
print(err("(?<=(a+))b"))
print(tryCatch(grepl("(?<=a+)b", "aab", perl = TRUE),
               warning = function(w) conditionMessage(w)))

show <- function(p, x) {
  m <- regexpr(p, x, perl = TRUE)
  print(c(m[1L], attr(m, "match.length"), attr(m, "capture.start"),
          attr(m, "capture.length")))
  print(gsub(p, "<\\0>", x, perl = TRUE))
}
show("(.+?)*", "aab")
show("(a+?)*b", "aab")
show("(\\w+?)*\\s", "ab cd")
show("(.*?)*", "aab")
show("(.+?){0,}", "aab")
show("(.+?)*+", "aab")
show("(a+?)(?#comment)*", "aaab")
show("(a+?){00,}", "aaab")
show("(a+?){2}", "aaab")
show("(a+?){1,}", "aaab")
show("(?:(.+?))*", "aab")
show("(?:(?:(.+?))*)*+", "aab")
show("(?<n>.+?)*", "aab")
show("(.+?)*?", "aab")
show("(a)?(?(1)b+?)*", "abb")
