# The unexpected ')' errors before the string is ever lexed: GNU R
# lexes on demand, so the unpaired surrogate after the error token
# warns nowhere, not even as "In addition:".
) "\uD800"
