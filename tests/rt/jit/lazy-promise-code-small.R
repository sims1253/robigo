# lazy-promise-code.R's small twin, for the torture steps (that case loads
# the compiler, `volume`): the same closures, compiled by GNU R 4.6.2's
# compiler::cmpfun and unserialized here (GNU R's
# serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     f = compiler::cmpfun(function(a, b) { x <- a + 1; g(x * b + a) }),
#     e = compiler::cmpfun(function(a) g(a + "z")),
#     s = compiler::cmpfun(function(a) w(sys.call())),
#     calls = compiler::cmpfun(function(a) g(length(sys.calls()))),
#     q = compiler::cmpfun(function(a) sb(a + 2 * a)),
#     p = compiler::cmpfun(function(a) c(gf(a * 10), ev(a * 100))),
#     n = compiler::cmpfun(function(a) ne(a + 1)))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below). The promises their byte code makes hold MAKEPROM's
# byte code, translated by robigo-rt at its first force; nothing of that
# shows.
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n14\nANSI_X3.4-1968\n531\n7\n1027\n253\n1026\n1\n26", 
  "2153\n1\na\n251\n1026\n1\n262153\n1\nb\n251\n254\n21\n6\n13\n17\n12\n20\n1\n16", 
  "\n2\n44\n3\n22\n4\n4\n23\n6\n29\n8\n38\n7\n1\n10\n6\n254\n0\n1\n262153\n1\n{\n2\n25", 
  "4\n244\n0\n6\n254\n0\n1\n262153\n2\n<-\n2\n254\n0\n1\n262153\n1\nx\n2\n254\n244", 
  "\n1\n6\n254\n0\n1\n262153\n1\n+\n2\n254\n0\n511\n2\n254\n0\n14\n1\n1\n0\n254\n0\n2", 
  "54\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n1\ng\n2\n254\n244\n3\n6\n254\n0\n1791", 
  "\n2\n254\n244\n4\n6\n254\n0\n1\n262153\n1\n*\n2\n254\n0\n1535\n2\n254\n0\n767\n0", 
  "\n254\n2\n254\n0\n511\n0\n254\n0\n254\n0\n254\n1\n511\n14\n14\n1\n1\n243\n1\n1\n1", 
  "535\n243\n0\n1\n2047\n243\n2\n21\n13\n12\n12\n20\n1\n20\n2\n46\n3\n20\n4\n44\n0\n", 
  "1\n6\n243\n3\n1\n1535\n1\n767\n243\n4\n1\n511\n13\n781\n12\nNA\n1\n1\n2\n2\n3\n3\n", 
  "4\n4\n0\n0\n0\n1026\n1\n262153\n5\nclass\n16\n1\n262153\n16\nexpressionsIn", 
  "dex\n254\n13\n781\n17\nNA\n1\n1\n2\n2\n3\n3\n5\n5\n5\n7\n7\n7\n7\n7\n7\n7\n1026\n25", 
  "59\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n1026\n511\n251", 
  "\n254\n21\n2\n13\n8\n12\n23\n1\n29\n2\n38\n0\n1\n4\n6\n254\n0\n2047\n2\n254\n244\n", 
  "0\n6\n254\n0\n1791\n2\n254\n0\n511\n2\n254\n0\n16\n1\n262153\n1\nz\n0\n254\n0\n2", 
  "54\n1\n2047\n21\n13\n8\n12\n20\n1\n16\n2\n44\n0\n1\n4\n243\n0\n1\n511\n16\n16\n1\n", 
  "262153\n1\nz\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n262153\n1", 
  "6\nexpressionsIndex\n254\n13\n781\n8\nNA\n0\n0\n0\n0\n0\n0\n0\n1026\n2559\n1", 
  "6\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n1026\n511\n251\n254", 
  "\n21\n2\n13\n8\n12\n23\n1\n29\n2\n38\n0\n1\n4\n6\n254\n0\n1\n262153\n1\nw\n2\n254\n", 
  "244\n0\n6\n254\n0\n1\n262153\n8\nsys.call\n0\n254\n0\n254\n1\n2815\n21\n13\n6", 
  "\n12\n23\n1\n38\n0\n1\n3\n243\n0\n1\n3071\n13\n781\n6\nNA\n0\n0\n0\n0\n0\n1026\n25", 
  "59\n16\n1\n262153\n16\nexpressionsIndex\n254\n13\n781\n8\nNA\n0\n0\n0\n0\n0", 
  "\n0\n0\n1026\n2559\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n", 
  "1026\n511\n251\n254\n21\n3\n13\n8\n12\n23\n1\n29\n2\n38\n0\n1\n4\n6\n254\n0\n204", 
  "7\n2\n254\n244\n0\n6\n254\n0\n1\n262153\n6\nlength\n2\n254\n244\n1\n6\n254\n0\n", 
  "1\n262153\n9\nsys.calls\n0\n254\n0\n254\n0\n254\n1\n2047\n21\n13\n14\n12\n12", 
  "3\n0\n13\n26\n1\n23\n2\n38\n3\n33\n39\n0\n1\n5\n243\n0\n1\n3327\n1\n3583\n243\n1\n", 
  "13\n781\n14\nNA\n0\n0\n0\n0\n0\n3\n3\n3\n3\n0\n0\n0\n0\n1026\n2559\n16\n1\n262153", 
  "\n16\nexpressionsIndex\n254\n13\n781\n8\nNA\n0\n0\n0\n0\n0\n0\n0\n1026\n2559", 
  "\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n1026\n511\n251\n2", 
  "54\n21\n3\n13\n8\n12\n23\n1\n29\n2\n38\n0\n1\n4\n6\n254\n0\n1\n262153\n2\nsb\n2\n2", 
  "54\n244\n0\n6\n254\n0\n1791\n2\n254\n0\n511\n2\n254\n244\n1\n6\n254\n0\n2303\n2", 
  "\n254\n0\n14\n1\n2\n2\n254\n0\n511\n0\n254\n0\n254\n0\n254\n1\n3839\n21\n13\n12\n", 
  "12\n20\n1\n16\n2\n20\n1\n46\n3\n44\n0\n1\n5\n243\n0\n1\n511\n14\n14\n1\n2\n243\n1\n", 
  "13\n781\n12\nNA\n1\n1\n2\n2\n1\n1\n3\n3\n0\n0\n0\n1026\n2559\n16\n1\n262153\n16\n", 
  "expressionsIndex\n254\n13\n781\n8\nNA\n0\n0\n0\n0\n0\n0\n0\n1026\n2559\n16\n", 
  "1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n1026\n511\n251\n254\n2", 
  "1\n5\n13\n23\n12\n123\n0\n22\n26\n1\n23\n2\n29\n4\n38\n3\n33\n23\n5\n29\n7\n38\n6\n", 
  "33\n39\n0\n1\n9\n6\n254\n0\n1\n262153\n1\nc\n2\n254\n244\n0\n6\n254\n0\n1\n26215", 
  "3\n2\ngf\n2\n254\n244\n1\n6\n254\n0\n2303\n2\n254\n0\n511\n2\n254\n0\n14\n1\n10\n", 
  "0\n254\n0\n254\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n2\nev\n2\n254\n244\n3\n6\n", 
  "254\n0\n2303\n2\n254\n0\n511\n2\n254\n0\n14\n1\n100\n0\n254\n0\n254\n0\n254\n1\n", 
  "4095\n1\n4351\n243\n0\n21\n13\n8\n12\n20\n1\n16\n2\n46\n0\n1\n4\n243\n1\n1\n511\n", 
  "14\n14\n1\n10\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n262153\n1", 
  "6\nexpressionsIndex\n254\n1\n4607\n243\n2\n21\n13\n8\n12\n20\n1\n16\n2\n46\n", 
  "0\n1\n4\n243\n3\n1\n511\n14\n14\n1\n100\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026", 
  "\n2559\n16\n1\n262153\n16\nexpressionsIndex\n254\n13\n781\n23\nNA\n0\n0\n0", 
  "\n0\n0\n3\n3\n3\n3\n3\n3\n0\n6\n6\n6\n6\n6\n6\n0\n0\n0\n0\n1026\n2559\n16\n1\n262153", 
  "\n16\nexpressionsIndex\n254\n1027\n253\n1026\n511\n251\n254\n21\n2\n13\n8", 
  "\n12\n23\n1\n29\n2\n38\n0\n1\n4\n6\n254\n0\n1\n262153\n2\nne\n2\n254\n244\n0\n6\n2", 
  "54\n0\n1791\n2\n254\n0\n511\n2\n254\n0\n14\n1\n1\n0\n254\n0\n254\n1\n4863\n21\n1", 
  "3\n8\n12\n20\n1\n16\n2\n44\n0\n1\n4\n243\n0\n1\n511\n14\n14\n1\n1\n13\n781\n8\nNA\n", 
  "1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n262153\n16\nexpressionsIndex\n254\n", 
  "13\n781\n8\nNA\n0\n0\n0\n0\n0\n0\n0\n1026\n2559\n16\n1\n262153\n16\nexpressio", 
  "nsIndex\n254\n1026\n1\n262153\n5\nnames\n16\n7\n262153\n1\nf\n262153\n1\ne", 
  "\n262153\n1\ns\n262153\n5\ncalls\n262153\n1\nq\n262153\n1\np\n262153\n1\nn\n", 
  "254\n"), collapse = "")))
g <- function(y) y
f <- cl$f
print(c(f(1, 2), f(3, 4)))
# an error inside the promise: its message and call, and the calls on the stack
e <- cl$e
r <- tryCatch(e(1), error = function(c) c)
print(conditionMessage(r)); print(conditionCall(r))
w <- function(y) { y; NULL }
s <- cl$s
print(s(1))
calls <- cl$calls
print(calls(1) > 0)
# substitute() and deparse of the promise
sb <- function(y) substitute(y)
q <- cl$q
print(q(1)); print(deparse(q(1)))
# forced from C: get() and eval() of the variable in the callee's frame
gf <- function(y) get("y")
ev <- function(y) eval(quote(y))
p <- cl$p
print(p(2))
# a promise never forced: its environment serialized, the promise in it
ne <- function(y) environment()
n <- cl$n
env <- n(5)
x <- serialize(env, NULL)
print(length(x))
print(paste(as.character(x), collapse = ""))
print(eval(quote(y), unserialize(x)))
