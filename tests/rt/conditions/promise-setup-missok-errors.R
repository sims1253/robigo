# R: eval.c:getvar. GETVAR_MISSOK reads at the depth limit whose read
# itself ends in an error or a missing argument, not in a force: robigo-rt
# runs those reads with byte code inactive as a whole (eval.rs
# get_var_keepmiss); their errors' calls and values are GNU R's. The JIT
# off: `fu`, `fm` and `k` are byte code, the rest is not. The closures,
# compiled by GNU R 4.6.2's compiler::cmpfun and unserialized here (GNU R's
# serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     fu = compiler::cmpfun(function() { options(expressions = 25L); 42L[y_unbound] }),
#     fm = compiler::cmpfun(function(x) { options(expressions = 25L); 42L[x] }),
#     k = compiler::cmpfun(function(d, f) if (d > 0L) k(d - 1L, f) else f()))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below).
invisible(compiler::enableJIT(0))
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n5\nUTF-8\n531\n3\n1027\n253\n254\n21\n3\n13\n20\n12\n2", 
  "3\n1\n34\n3\n31\n4\n38\n2\n4\n16\n6\n104\n5\n19\n92\n7\n84\n5\n1\n9\n6\n254\n0\n1\n2", 
  "62153\n1\n{\n2\n254\n244\n0\n6\n254\n0\n1\n262153\n7\noptions\n2\n1\n262153\n", 
  "11\nexpressions\n0\n13\n1\n25\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1\n262153\n", 
  "1\n[\n2\n254\n0\n13\n1\n42\n2\n254\n0\n1\n262153\n9\ny_unbound\n0\n254\n0\n254", 
  "\n1\n767\n243\n0\n13\n13\n1\n25\n1\n1023\n243\n1\n13\n13\n1\n42\n1\n1535\n13\n78", 
  "1\n20\nNA\n2\n2\n2\n2\n2\n2\n2\n2\n2\n6\n6\n5\n5\n5\n7\n7\n5\n5\n5\n1026\n1\n262153\n", 
  "5\nclass\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n1026\n1\n", 
  "262153\n1\nx\n251\n254\n21\n3\n13\n20\n12\n23\n1\n34\n3\n31\n4\n38\n2\n4\n16\n6\n", 
  "104\n5\n19\n92\n7\n84\n5\n1\n9\n6\n254\n0\n511\n2\n254\n244\n0\n6\n254\n0\n767\n2", 
  "\n1023\n0\n13\n1\n25\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1279\n2\n254\n0\n13\n1\n", 
  "42\n2\n254\n0\n2047\n0\n254\n0\n254\n1\n767\n243\n0\n13\n13\n1\n25\n1\n1023\n24", 
  "3\n1\n13\n13\n1\n42\n1\n2047\n13\n781\n20\nNA\n2\n2\n2\n2\n2\n2\n2\n2\n2\n6\n6\n5\n5", 
  "\n5\n7\n7\n5\n5\n5\n1026\n1791\n16\n1\n262153\n16\nexpressionsIndex\n254\n1", 
  "027\n253\n1026\n1\n262153\n1\nd\n251\n1026\n1\n262153\n1\nf\n251\n254\n21\n5", 
  "\n13\n24\n12\n20\n1\n16\n2\n56\n3\n3\n0\n19\n23\n4\n29\n6\n29\n7\n38\n5\n1\n23\n8\n3", 
  "8\n9\n1\n11\n6\n254\n0\n1\n262153\n2\nif\n2\n254\n244\n0\n6\n254\n0\n1\n262153\n", 
  "1\n>\n2\n254\n0\n2303\n2\n254\n0\n13\n1\n0\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1\n", 
  "262153\n1\nk\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n1\n-\n2\n254\n0\n2303\n2\n2", 
  "54\n0\n13\n1\n1\n0\n254\n2\n254\n0\n2559\n0\n254\n2\n254\n244\n3\n6\n254\n0\n255", 
  "9\n0\n254\n0\n254\n1\n2303\n13\n13\n1\n0\n243\n0\n1\n3327\n243\n1\n21\n13\n8\n12", 
  "\n20\n1\n16\n2\n45\n0\n1\n4\n243\n2\n1\n2303\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2", 
  "\n2\n0\n0\n0\n1026\n1791\n16\n1\n262153\n16\nexpressionsIndex\n254\n21\n13", 
  "\n4\n12\n20\n0\n1\n3\n1\n2559\n243\n1\n13\n781\n4\nNA\n1\n1\n1\n1026\n1791\n16\n1", 
  "\n262153\n16\nexpressionsIndex\n254\n1\n2559\n243\n3\n13\n781\n24\nNA\n1\n", 
  "1\n2\n2\n3\n3\n0\n0\n0\n5\n5\n5\n5\n5\n5\n5\n5\n5\n9\n9\n9\n9\n9\n1026\n1791\n16\n1\n2", 
  "62153\n16\nexpressionsIndex\n254\n1026\n1\n262153\n5\nnames\n16\n3\n262", 
  "153\n2\nfu\n262153\n2\nfm\n262153\n1\nk\n254\n"), collapse = "")))
fu <- cl$fu
fm <- cl$fm
k <- cl$k
r <- tryCatch(k(30L, fu), error = function(e) e)
options(expressions = 5000L)
print(conditionMessage(r))
print(conditionCall(r))
r <- tryCatch(k(30L, fm), error = function(e) e)
options(expressions = 5000L)
print(if (inherits(r, "condition")) conditionMessage(r) else r)
