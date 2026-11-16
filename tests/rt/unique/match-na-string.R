# NA_character_ and the string "NA": the same bytes, never equal. GNU R's
# NA_STRING is a cached CHARSXP, so Seql's pointer test tells them apart;
# match's scalar case (match5) compares with Seql, the hashed case with
# sequal.
print(match(NA_character_, "NA"))
print(match("NA", NA_character_))
print(match(NA_character_, c("NA", NA)))
print(match("NA", c(NA, "NA")))
print(NA_character_ %in% c("NA", "b"))
print("NA" %in% NA_character_)
print(match(c(NA_character_, "a"), c("NA", "a")))
print(match(c("NA", "a"), c(NA, "a")))
print(identical(NA_character_, "NA"))
