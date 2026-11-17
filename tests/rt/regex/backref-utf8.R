# Back references in a UTF-8 locale: grep.c's paths (bytes, wide, and the
# multibyte tre_regexec of strsplit and regexec for unmarked input) answer
# as GNU R does. (list.files' multibyte match is not ported: it refuses.)
x <- c("aa", "ab", "éé", "xééy", "abcabc", ".", "..", "sub/deeper", "a.txt")
grep("(.)\\1", x)
grepl("(.)\\1", x)
regexpr("(.)\\1", x)
gregexpr("(.)\\1", x)[[4]]
sub("(.)\\1", "<\\1>", x)
gsub("(.)\\1", "<\\1>", x)
regexpr("^(.).*\\1$", x)
grepl("(.+)\\1", x)
regexec("(a|é)\\1", x)[[3]]
strsplit(x, "(.)\\1")
grepl("(.)\\1", x, useBytes = TRUE)
grepl("(.)\\1", x, ignore.case = TRUE)
# all ASCII, nothing marked: strsplit and regexec match multibyte strings
a <- c("aa", "ab", "abcabc", ".", "..", "sub/deeper", "a.txt", "xxyyzz")
strsplit(a, "(.)\\1")
regexec("(.)\\1", a)
regexec("^(.).*\\1$", a)
strsplit("aabbcc", "(.)\\1")
