# regexec: the match and its sub-expressions, through TRE and fixed
# strings, with NA and unmatched inputs. (perl = TRUE is R code over
# regexpr that needs cbind.)
x <- c("ab", "a", "x", NA, "")
print(regexec("(a)(b)?", x))
print(regexec("a", x))
print(regexec("(é+)(.)", c("xééy", "é")))
print(regexec("(a|b)*c", "ababc"))
print(regexec("A(B)", "xab", ignore.case = TRUE))
print(regexec("a.", "xaé", fixed = TRUE))
print(regexec("é", "xé", fixed = TRUE))
print(regexec("é", "aé", useBytes = TRUE))
print(regexec("b", c(x = "abc")))
print(regexec("a", character(0)))
