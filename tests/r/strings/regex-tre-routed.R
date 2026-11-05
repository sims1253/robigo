# The TRE constructs robigo sends to GNU R's TRE (issue #423): minimal
# repetitions, back-references, word boundaries, approximate matching and
# ambiguous submatches, through each regex builtin, on ASCII and non-ASCII
# text and with useBytes.
f <- function(p, x, ...) {
  m <- gregexpr(p, x, ...)[[1]]
  paste0(m, "+", attr(m, "match.length"))
}

# minimal repetitions inside alternations
print(f("a.*?b|a.*", "aXbYb"))
print(f("x*?y|x+", "xxxy"))
print(f("a+?|b", "aaab"))
print(f("a{1,3}?b|a", "aaab"))
re <- paste0("(?:(?:\\x{001b}\\[)|\\x{009b})",
  "(?:(?:[0-9]{1,3})?(?:(?:;[0-9]{0,3})*)?[A-M|f-m])", "|\\x{001b}[A-M]",
  "|\\x{001b}\\]8;.*?;.*?\\x{001b}\\\\", "|\\x{001b}\\]8;.*?;.*?\\x{0007}")
txt0 <- "1\033]8;;https://ex.com\007text\033]8;;\0072"
txt <- paste0(txt0, gsub("\007", "\033\\", txt0, fixed = TRUE))
print(f(re, txt))
print(gsub(re, "", txt))
print(regmatches(txt, gregexpr(re, txt))[[1]])

# minimal repetitions in each builtin
x <- c("aXbYb", "<a><bb>", "no", NA, "éaébé")
print(regexpr("a.*?b", x))
print(sub("<.+?>", "[]", x))
print(gsub("<.+?>", "[]", x))
print(gsub("(.+?)b", "\\1|", x))
print(strsplit(c("a1b22c333", "é1é22"), "[0-9]+?"))
print(regexec("(a.*?)(b+)", x))
print(grepl("a.*?b", x))
print(regexpr("A.*?B", x, ignore.case = TRUE))
print(regexpr("a.*?b", x, useBytes = TRUE))

# back-references
print(gsub("(a+)\\1", "<\\1>", c("aaaa", "aaa", "abab")))
print(regexpr("(.)\\1", c("abba", "éé", "xyz")))
print(grepl("^(.+)\\1$", c("abab", "aba", "éé")))

# word boundaries, at the ends of the (sub)string and around non-ASCII
print(gsub("\\b", "|", c("ab cd", "é b", "")))
print(gsub("\\B", "-", c("abc", "a b", "éé")))
print(gsub("\\<", "<", "one two"))
print(gsub("\\>", ">", "one two"))
print(f("\\bé", "é aé é"))
print(strsplit("one two  three", "\\b"))

# approximate matching syntax
print(regexpr("(abc){~1}", c("xabcx", "xabdx", "xyzx")))
print(regexpr("(foo){+1-1}", c("fooo", "fo", "bar")))

# submatches of alternations and repeated groups (POSIX rules)
print(regmatches("abcd", regexec("(a|ab)(c|bcd)(d*)", "abcd"))[[1]])
print(regmatches("xyz", regexec("(x|xy)(z|yz)", "xyz"))[[1]])
print(regmatches("abab", regexec("(ab|a)+", "abab"))[[1]])
print(regmatches("aaa", regexec("(a|aa)*", "aaa"))[[1]])
print(sub("(a|ab)(c|bcd)", "[\\1][\\2]", "abcd"))
print(gsub("(ab|a)(b*)", "<\\1,\\2>", "abbab"))

# errors from TRE
r <- tryCatch(regexpr("a{2,1}?", "a"), error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w))
print(r)
