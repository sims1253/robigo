# R: names.c:installTrChar — a name a runtime caller installs (an assignment
# target, as.symbol, assign, ...) rejects the empty string and names over
# 10000 bytes, while the parser itself keeps `'' <- 123` as a string lhs.
cat(tryCatch(eval(parse(text = "'' <- 123")), error = conditionMessage), "\n")
cat(tryCatch(eval(parse(text = '"noname" <- 7; noname')), error = conditionMessage), "\n")
cat(tryCatch(as.symbol(""), error = conditionMessage), "\n")
cat(tryCatch(as.name(""), error = conditionMessage), "\n")
cat(tryCatch(as.symbol("x"), error = conditionMessage), "\n")
print(parse(text = "'' <- 123"))

cat(tryCatch(assign("", 42), error = conditionMessage), "\n")
assign("ok1", 42)
cat(ok1, "\n")
cat(tryCatch(makeActiveBinding("", function(x) 1), error = conditionMessage), "\n")
cat(tryCatch(delayedAssign("", 1), error = conditionMessage), "\n")

# 10000 bytes is allowed, 10001 is not.
long <- strrep("a", 10001)
cat(tryCatch(eval(call("<-", long, 1)), error = conditionMessage), "\n")
ok2 <- eval(call("<-", strrep("b", 10000), 2))
cat(get(strrep("b", 10000)), "\n")
cat(tryCatch(assign(strrep("c", 10001), 3), error = conditionMessage), "\n")
