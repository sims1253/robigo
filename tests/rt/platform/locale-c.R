# The locale from the environment (main.c's setlocale(<category>, "") at
# startup, platform.c's R_check_locale): this case runs under LC_ALL=C
# (its .locale sidecar), locale-utf8.R the same lines under C.UTF-8.
# l10n_info() and serialize's version 3 header, whose native encoding is
# R_nativeEncoding() (nl_langinfo(CODESET): "ANSI_X3.4-1968" in C).
print(l10n_info())
x <- serialize(1L, NULL, version = 3L)
print(length(x))
print(x)
y <- unserialize(x)
print(y)
print(nchar("abc", "chars"))
