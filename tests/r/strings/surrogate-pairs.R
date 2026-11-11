# Surrogate halves escape-by-escape: a high half followed by a low
# half combines into one code point (gram.c pushes each escape's raw
# value; util.c:wcstoutf8 combines the pair at the end). An unpaired
# half warns (twice per statement: the REPL trial-parses and then
# really parses) and encodes as its own value (Rwcrtomb32), which is
# not valid UTF-8 for R's multibyte readers.
print(utf8ToInt("\uD83D\uDE00"))
print(utf8ToInt("\u{D83D}\u{DE00}"))
print(utf8ToInt("\U0000D83D\U0000DE00"))
print(utf8ToInt("\uD83D\U{DE00}"))
print(utf8ToInt("\U{D83D}\uDE00"))
print(utf8ToInt("\uD800\uDC00"))
print(utf8ToInt("\uDBFF\uDFFF"))
print(nchar("\u{D83D}\u{DE00}", type = "bytes"))
print(charToRaw("\u{D83D}\u{DE00}"))
# a character between the halves keeps them unpaired
print(charToRaw("\uD83D\u0041\uDE00"))
print(try(utf8ToInt("\uD83D\u0041\uDE00"), silent = TRUE))
# unpaired halves
print(charToRaw("\uD800"))
print(charToRaw("\uDE00"))
print(utf8ToInt("\uD83D"))
print(try(nchar("\uD83D\u0041", type = "chars"), silent = TRUE))
# paste0 of two invalid-marked halves keeps them invalid for
# utf8ToInt: tracked in paste-invalid-utf8.R until paste carries
# encodings.
print(utf8ToInt("\uD83D\uDE00") == 128512)
# the trial parse of an incomplete statement reaches the escape and
# fires the warning before the statement completes (Rf_ReplIteration
# prints pending warnings at the top of each iteration)
print(charToRaw("\uD800"
))
# warn = 1 prints each warning as it fires: the trial parse's warning
# is out before the statement's own output
options(warn = 1)
print(charToRaw("\uD83D\uDE00"))
print(charToRaw("\uD800"))
