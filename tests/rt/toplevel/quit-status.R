# quit() ends the session at once with its status: nothing after it runs.
cat("before\n")
quit("no", 3)
cat("after\n")
