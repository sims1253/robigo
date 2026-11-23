# The same inside a call: the unterminated argument list errors on the
# string it just lexed, and the warning trails the error.
f(1 "\uD800")
