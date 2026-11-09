# Surrogate pairs in Unicode escapes combine into one code point, in
# every spelling: gram.c pushes each escape as a wchar_t and the pairing
# happens in the wide -> UTF-8 conversion (util.c:wcstoutf8). An
# unpaired surrogate warns ("unpaired surrogate Unicode point %x") and
# becomes U+FFFD, one warning per escape.
utf8ToInt("\uD83D\uDE00")
utf8ToInt("\u{D83D}\u{DE00}")
utf8ToInt("\U0000D83D\U0000DE00")
utf8ToInt("\uD83D\U{DE00}")
utf8ToInt("\U{D83D}\uDE00")
utf8ToInt("\uD800\uDC00")
utf8ToInt("\uDBFF\uDFFF")
utf8ToInt("\uD83D\uDE00\uD83D\uDE00")
charToRaw("\u{D83D}\u{DE00}")
nchar("\uD83D\uDE00", type = "chars")
# unpaired surrogates: one warning per escape, U+FFFD each
utf8ToInt("\uD800")
utf8ToInt("\u{DC00}")
utf8ToInt("\U0000DFFF")
utf8ToInt("\uD800\uD800\uD800")
utf8ToInt("\uDC00\uD800")
# a high surrogate followed by an ordinary character does not pair
utf8ToInt("\uD83Dx")
substr("\uD83D\uDE00", 1, 1)
