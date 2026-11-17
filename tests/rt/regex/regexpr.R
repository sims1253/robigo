# regexpr: TRE on bytes and on wide characters, fixed strings, PCRE with
# named and unset capture groups, NA and the attributes. (The capture
# matrices print through their pieces.)
shw <- function(r) {
    for (a in c("capture.start", "capture.length")) {
        m <- attr(r, a)
        attr(r, a) <- NULL
        print(attr(m, "dim"))
        print(attr(m, "dimnames"))
        print(c(m))
    }
    print(r)
}
x <- c("banana", "apple", NA, "", "cherry")
print(regexpr("an", x))
print(regexpr("a+", x))
print(regexpr("x*", x))
print(regexpr("p{2}", x))
print(regexpr("[[:upper:]]", c("abC", "D", "e")))
print(regexpr("A", c("bab", "BAB"), ignore.case = TRUE))
print(regexpr("é+", c("xééy", "é", "e")))
print(regexpr("y", c("éy", "y")))
print(regexpr("é", "aé", useBytes = TRUE))
print(regexpr(".", "aé", fixed = TRUE))
print(regexpr("é", c("aé", "é", "a"), fixed = TRUE))
print(regexpr("an", x, fixed = TRUE))
shw(regexpr("a(n)", x, perl = TRUE))
print(regexpr("(?i)AN", x, perl = TRUE))
print(regexpr("é.", "aéb", perl = TRUE))
print(regexpr("a", character(0)))
print(regexpr("b", c(x = "abc", y = "b")))
shw(regexpr("a(b)?(?<nm>c)?", c("a", "abc", NA), perl = TRUE))
shw(regexpr("(?<first>[[:upper:]][[:lower:]]+) (?<last>[[:upper:]][[:lower:]]+)",
    c("Ada Lovelace", "nobody", "Grace Hopper"), perl = TRUE))
shw(regexpr("(é)(x)?", "aé", perl = TRUE))
