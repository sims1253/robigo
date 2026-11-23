# Compiles reuse the isolate's spare buffers (compile.rs's Scratch): nested
# compiles, a compile abandoned to force a promise first, and compiles of
# very different sizes in a row give what fresh buffers give.
# nested compiles: the promise codes of arguments, a function made in code
g <- function(a, b) a * 10 + b
f <- function(y) g(y + 1, y * 2)
for (i in 1:3) print(f(i))
mk <- function(k) function(a) a + k
for (i in 1:3) print(mk(i)(10))
# a compile that forces a promise and starts again (cenv's Need::Force)
delayedAssign("h", function(x) x - 1)
u <- function(z) h(z) + h(z * 2)
for (i in 1:3) print(u(i))
# one-shot compiles of growing size, then one past what is kept spare
for (i in 1:5) print(eval(parse(text = paste(rep("1", i), collapse = " + "))))
big <- paste0("c(", paste(seq_len(3000), collapse = ", "), ")")
print(length(eval(parse(text = big))))
print(eval(parse(text = "2 * 21")))
