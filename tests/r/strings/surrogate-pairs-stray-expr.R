# The unexpected second expression is the string itself: GNU R lexed
# it before reporting, so its warning trails the parse error as
# "In addition:".
1 "\uD800"
