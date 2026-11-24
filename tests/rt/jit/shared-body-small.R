# shared-body.R's small twin, for the torture steps (that case loads the
# compiler, `volume`): its maker closure compiled by GNU R 4.6.2's
# compiler::cmpfun and unserialized here (GNU R's
# serialize(cl, NULL, ascii = TRUE) of
#   cl <- list(mk = compiler::cmpfun(function(a) function(x, y = a) x + y + a))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below).
# Closures `function` makes from one byte-code body share it, and with it
# robigo-rt's translation of it.
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n14\nANSI_X3.4-1968\n531\n1\n1027\n253\n1026\n1\n26", 
  "2153\n1\na\n251\n254\n21\n1\n13\n4\n12\n41\n1\n1\n3\n6\n254\n0\n1\n262153\n8\nfu", 
  "nction\n2\n254\n2\n1\n262153\n1\nx\n0\n251\n2\n1\n262153\n1\ny\n0\n511\n0\n254", 
  "\n2\n254\n6\n254\n0\n1\n262153\n1\n+\n2\n254\n6\n254\n0\n1535\n2\n254\n0\n1023\n", 
  "2\n254\n0\n1279\n0\n254\n2\n254\n0\n511\n0\n254\n2\n254\n0\n254\n0\n254\n19\n19", 
  "\n3\n1026\n1023\n251\n1026\n1279\n511\n254\n21\n2\n13\n12\n12\n20\n1\n20\n2\n4", 
  "4\n3\n20\n4\n44\n0\n1\n6\n6\n254\n0\n1535\n2\n254\n244\n0\n6\n254\n0\n1535\n2\n25", 
  "4\n0\n1023\n2\n254\n0\n1279\n0\n254\n2\n254\n0\n511\n0\n254\n1\n1023\n1\n1279\n", 
  "243\n0\n1\n511\n13\n781\n12\nNA\n1\n1\n2\n2\n3\n3\n4\n4\n0\n0\n0\n1026\n1\n262153", 
  "\n5\nclass\n16\n1\n262153\n16\nexpressionsIndex\n254\n254\n13\n781\n4\nNA", 
  "\n0\n0\n0\n1026\n1791\n16\n1\n262153\n16\nexpressionsIndex\n254\n1026\n1\n", 
  "262153\n5\nnames\n16\n1\n262153\n2\nmk\n254\n"), collapse = "")))
mk <- cl$mk
f1 <- mk(1); f10 <- mk(10)
print(c(f1(1), f10(1), f1(1, 2), f10(1, 2)))
print(identical(body(f1), body(f10)))
print(typeof(.Internal(bodyCode(f1))))
# many closures from one literal
fs <- lapply(1:5, function(i) mk(i))
print(vapply(fs, function(f) f(0), 0))
# one closure's formals rewritten: its own code, the others unchanged
g <- mk(2)
formals(g) <- alist(x = , y = 100, z = 0)
print(c(g(1), f1(1), f10(1)))
# body<- and environment<- on one closure: its own body (source, run as
# GNU R runs it), the others unchanged
h <- mk(3)
body(h) <- quote(x * 1000)
print(c(h(2), f1(1), f10(1)))
k <- mk(4)
environment(k) <- list2env(list(a = 50))
print(c(k(1), f1(1), f10(1)))
