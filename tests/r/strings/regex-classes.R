# POSIX classes on ASCII-only and non-ASCII text (the former compile
# ASCII-restricted classes)
a <- c("abc 123", "  Hello_World!", "x9 y8\tz7", "", NA, "UPPER lower", "a.b-c")
u <- c("café über", "١٢ 12", "αβγ abc", "naïve")
classes <- c("[[:alpha:]]+", "[[:alnum:]]+", "[[:digit:]]+", "[[:upper:]]+", "[[:lower:]]+",
             "[[:space:]]+", "[[:punct:]]+", "[[:blank:]]*([[:alnum:]]+)", "[^[:alnum:]]+",
             "\\w+", "\\W+", "\\d+", "\\s+", "[[:print:]]+", "[[:graph:]]+", "\\<[a-z]", "\\bo")
for (p in classes) {
  cat(p, "\n")
  print(gsub(p, "<\\0>", a))
  print(gsub(p, "<&>", u))
  print(regexpr(p, a))
  print(grepl(p, u))
}
sub("[[:blank:]]*([[:alnum:]]+)", "\\1", "  R_DEFAULT_PACKAGES")
gsub("[[:upper:]]", "_", c("aBcD", "Été"), ignore.case = TRUE)
grepl("^[[:alpha:]]+$", c("abc", "ab1", "été"))
strsplit(c("a1b22c333", "xéy"), "[[:digit:]]+")
regmatches("one two  three", gregexpr("[[:alpha:]]+", "one two  three"))
toupper(gsub("(\\w)(\\w*)", "\\U\\1\\L\\2", "hello world", perl = TRUE))
