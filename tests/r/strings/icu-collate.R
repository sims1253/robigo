# icuSetCollate / icuGetCollate and the collation they set (R: util.c).
x <- c("b", "A", "a", "B", "_x", "1", "é", "e", "f", "ä", "z", "Z", "10", "9", "a b", "ab")
# the collator is opened at the first comparison of two different strings
icuGetCollate()
sort(x)
icuGetCollate()
icuGetCollate("valid")
"a" < "B"
"B" < "a"
"a" == "a"
order(x)
rank(c("b", "a", "B", "A"))
min(x); max(x); range(x)
pmin(c("a", "B"), c("A", "b"))
ls(list2env(list(b = 1, A = 2, a = 3, B = 4)))

# locales and their tailorings
icuSetCollate(locale = "sv")
icuGetCollate()
sort(c("ä", "z", "a", "ö", "o"))
icuSetCollate(locale = "de@collation=phonebook")
icuGetCollate(); icuGetCollate("valid")
sort(c("Ärger", "Alpha", "Affe", "Azur"))
icuSetCollate(locale = "en_US")
icuGetCollate()
icuSetCollate(locale = "xx_YY")
icuGetCollate(); icuGetCollate("valid")
icuSetCollate(locale = "ASCII")
icuGetCollate()
sort(x)
"a" < "B"
icuSetCollate(locale = "none")
icuGetCollate()
sort(x)
icuSetCollate(locale = "default")
icuGetCollate()
sort(x)

# attributes
icuSetCollate(case_first = "upper")
sort(x)
icuSetCollate(case_first = "default", numeric = "on")
sort(c("a10", "a9", "a100", "a1"))
icuSetCollate(numeric = "off", strength = "primary")
sort(c("b", "a", "A", "á"))
rank(c("a", "A", "á", "b"))
"a" < "A"; "a" == "A"
icuSetCollate(strength = "default", alternate_handling = "shifted")
sort(c("a-b", "ab", "a b", "a_c", "aa"))
icuSetCollate(alternate_handling = "default")
# names or values not in the table are ignored
icuSetCollate(case_first = "bogus", bogus = "on")
sort(c("B", "a"))

# canonically equivalent strings collate equal
sort(c("é", "é", "f", "e"))
rank(c("é", "é"))
sort(c("中", "日", "本", "a", "А", "α"))

# the C locale disables the collator; another locale resets it
invisible(Sys.setlocale("LC_COLLATE", "C"))
icuGetCollate()
sort(x)
"a" < "B"
invisible(Sys.setlocale("LC_COLLATE", "C.UTF-8"))
icuGetCollate()
sort(x)
icuGetCollate()
capabilities("ICU")

# errors
r <- try(icuSetCollate("x"))
r <- try(icuSetCollate(locale = 1))
r <- try(icuSetCollate(locale = c("a", "b")))
r <- try(icuSetCollate(locale = NA_character_))
r <- try(icuGetCollate("foo"))
r <- try(.Internal(icuGetCollate(3L)))
r <- try(.Internal(icuGetCollate(NA)))
