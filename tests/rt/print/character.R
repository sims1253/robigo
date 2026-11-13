# printVector for strings: quoted, padded to the widest, escapes, UTF-8,
# NA_character_ (printed NA, unquoted) against the string "NA".
paste0("a", 1:40)
paste0("x", 1:3, "\t", "y")
paste("é", 1:12, "日本")
NA_character_
"NA"
paste0("N", "A")
paste0("tab", "\t", "quote\"", "back\\slash", "\n")
