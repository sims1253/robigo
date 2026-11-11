# R_Parse1's checkForPlaceholder fails only in the real parse, after
# the trial parse accepted the statement: both parses lexed the string,
# so the unpaired surrogate warns twice.
f("\uD800", _)
