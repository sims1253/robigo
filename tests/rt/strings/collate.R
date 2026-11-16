# String orderings under ICU's root collator, GNU R's in the oracle's C.UTF-8
# (util.c's Scollate through relop.c, summary.c and options.c).

# case: lower before upper at the third level, letters before the second
print(c("a" < "B", "B" < "b", "A" < "b", "a" <= "A", "A" >= "a", "a" > "A"))
# accents: second level; a precomposed letter and its decomposition are equal
print(c("e" < "\u00e9", "\u00e9" < "f", "\u00e9" < "E", "\u00e8" < "\u00e9", "\u00e9" > "\u00ca"))
print(c("e\u0301" < "\u00e9", "e\u0301" > "\u00e9", "e\u0301" <= "\u00e9", "e\u0301" == "\u00e9"))
# digits compare as text; punctuation and spaces are not ignored
print(c("10" < "9", "1" < "a", "\u0661" > "1", "\u0661" < "2", "\uff11" > "1"))
print(c("_x" < "a", "a b" < "ab", "a-b" < "ab", "a.b" < "a-b", "a_b" < "a b"))
# empty strings and NA
print(c("" < "a", "" < " ", "a" < NA, NA > "", "" >= ""))
# other scripts: Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai, CJK (by
# radical and stroke), Hangul (syllables as their jamo), kana, emoji
s <- c("\u03b1", "\u0430", "\u05d0", "\u0627", "\u0915", "\u0e01", "\u4e00", "\u7e37", "\U000218a2",
       "\uac00", "\u1100\u1161", "\u3042", "\u30a2", "\U0001F600", "z", "\ufffd")
print(vapply(s, function(t) sum(s < t), 0, USE.NAMES = FALSE))
print(vapply(s, function(t) sum(s == t | (s <= t & s >= t)), 0, USE.NAMES = FALSE))
# encodings: UTF-8 against invalid native bytes, read as U+FFFD
print(c("\xff" < "\ufffd", "\xff" > "\ufffd", "\xff\xfe" <= "\ufffd\ufffd", "a\xffb" < "a\ufffdc"))
# a rank by comparison of strings built from pieces of each kind
p <- c("", "a", "A", "\u00e1", "a\u0301", "b", "1", "-", " ", "\u00df", "ss", "\u0153", "oe", "\u00f8",
       "\u0436", "\u4e09", "\uac01")
x <- character()
for (q in c("", "a", "\u00e0", "Z", "2")) x <- c(x, paste0(p, q))
print(length(x))
print(vapply(x, function(t) sum(x < t), 0, USE.NAMES = FALSE))
# min, max and pmin
print(c(min(x), max(x), min(s), max(s)))
print(max(c("b", NA, "a")))
print(min(c("b", NA, "a"), na.rm = TRUE))
print(max(character(), "\u00e9", "e"))
print(pmin(c("a", "B", NA, "\u00e9"), c("A", "b", "c", "e")))
print(pmin(c("a", "B", NA, "\u00e9"), c("A", "b", "c", "e"), na.rm = TRUE))
# options() lists its options by name
n <- names(options())
print(n[n %in% c("digits", "OutDec", "scipen", "warn", "echo", "keep.source", "width", "CBoundsCheck",
                 "add.smooth", "check.bounds", "nwarnings", "PCRE_use_JIT")])
