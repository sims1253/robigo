# Input ends while a statement is open: the trial lex of the first
# chunk already fired the warning (printed when the next chunk starts),
# then the incomplete buffer is an error at end of input.
x <- 1
print(charToRaw("\uD800"
