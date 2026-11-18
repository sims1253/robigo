# gcinfo(TRUE)'s switch: the value is the old state. Collections between
# the two calls report on stderr, so the recording depends on where they
# happen (`.requires` collection-schedule: not under gctorture or retry
# torture).
gcinfo(TRUE)
gcinfo(FALSE)
