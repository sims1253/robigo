# R: eval.c:DO_GETVAR, eval.c:getvar. GETVAR_MISSOK (here `42L[x]`'s
# subscript) reads through getvar, not bcEval's inlined force: a promise it
# forces whose evaluation overflows is pending, so it is marked
# interrupted (the restart warning on the retry), unlike a GETVAR's
# (promise-setup-overflow.R). The JIT off: `f` and `k` are byte code, the
# rest is not. The closures, compiled by GNU R 4.6.2's compiler::cmpfun and
# unserialized here (GNU R's serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     f = compiler::cmpfun(function(x) { last <<- environment(); options(expressions = 25L); 42L[x] }),
#     k = compiler::cmpfun(function(d) if (d > 0L) k(d - 1L) else f(1L + 0L)))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below).
invisible(compiler::enableJIT(0))
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n5\nUTF-8\n531\n2\n1027\n253\n1026\n1\n262153\n1\nx\n2", 
  "51\n254\n21\n5\n13\n27\n12\n23\n1\n38\n2\n95\n3\n4\n23\n5\n34\n7\n31\n8\n38\n6\n4\n", 
  "16\n10\n104\n9\n26\n92\n11\n84\n9\n1\n13\n6\n254\n0\n1\n262153\n1\n{\n2\n254\n24", 
  "4\n0\n6\n254\n0\n1\n262153\n3\n<<-\n2\n254\n0\n1\n262153\n4\nlast\n2\n254\n244", 
  "\n1\n6\n254\n0\n1\n262153\n11\nenvironment\n0\n254\n0\n254\n2\n254\n244\n2\n6", 
  "\n254\n0\n1\n262153\n7\noptions\n2\n1\n262153\n11\nexpressions\n0\n13\n1\n2", 
  "5\n0\n254\n2\n254\n244\n3\n6\n254\n0\n1\n262153\n1\n[\n2\n254\n0\n13\n1\n42\n2\n2", 
  "54\n0\n511\n0\n254\n0\n254\n1\n1535\n243\n1\n1\n1279\n243\n0\n1\n1791\n243\n2\n", 
  "13\n13\n1\n25\n1\n2047\n243\n3\n13\n13\n1\n42\n1\n511\n13\n781\n27\nNA\n2\n2\n2\n", 
  "2\n4\n4\n4\n6\n6\n6\n6\n6\n6\n6\n6\n6\n10\n10\n9\n9\n9\n11\n11\n9\n9\n9\n1026\n1\n262", 
  "153\n5\nclass\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n102", 
  "6\n1\n262153\n1\nd\n251\n254\n21\n6\n13\n24\n12\n20\n1\n16\n2\n56\n3\n3\n0\n17\n2", 
  "3\n4\n29\n6\n38\n5\n1\n23\n7\n29\n9\n38\n8\n1\n11\n6\n254\n0\n1\n262153\n2\nif\n2\n", 
  "254\n244\n0\n6\n254\n0\n1\n262153\n1\n>\n2\n254\n0\n2815\n2\n254\n0\n13\n1\n0\n0", 
  "\n254\n2\n254\n244\n1\n6\n254\n0\n1\n262153\n1\nk\n2\n254\n244\n2\n6\n254\n0\n1\n", 
  "262153\n1\n-\n2\n254\n0\n2815\n2\n254\n0\n13\n1\n1\n0\n254\n0\n254\n2\n254\n244", 
  "\n3\n6\n254\n0\n1\n262153\n1\nf\n2\n254\n244\n4\n6\n254\n0\n1\n262153\n1\n+\n2\n2", 
  "54\n0\n13\n1\n1\n2\n254\n0\n13\n1\n0\n0\n254\n0\n254\n0\n254\n1\n2815\n13\n13\n1\n", 
  "0\n243\n0\n1\n3583\n243\n1\n21\n13\n8\n12\n20\n1\n16\n2\n45\n0\n1\n4\n243\n2\n1\n2", 
  "815\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n26215", 
  "3\n16\nexpressionsIndex\n254\n1\n4095\n243\n3\n21\n13\n4\n12\n16\n1\n1\n4\n2", 
  "43\n4\n13\n13\n1\n1\n243\n3\n13\n781\n4\nNA\n2\n2\n2\n1026\n2559\n16\n1\n262153", 
  "\n16\nexpressionsIndex\n254\n13\n781\n24\nNA\n1\n1\n2\n2\n3\n3\n0\n0\n0\n5\n5\n", 
  "5\n5\n5\n5\n5\n8\n8\n8\n8\n8\n8\n8\n1026\n2559\n16\n1\n262153\n16\nexpressions", 
  "Index\n254\n1026\n1\n262153\n5\nnames\n16\n2\n262153\n1\nf\n262153\n1\nk\n2", 
  "54\n"), collapse = "")))
f <- cl$f
k <- cl$k
last <- NULL
print(tryCatch(k(30L), error = function(e) conditionMessage(e)))
options(expressions = 5000L)
W <- 0L
print(tryCatch(withCallingHandlers(last$x, warning = function(w) {
    W <<- W + 1L
    invokeRestart("muffleWarning")
}), error = function(e) conditionMessage(e)))
print(W)
