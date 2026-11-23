# INTERIM(S1): closes with X's vm.rs (base_guard). R: eval.c:SymbolValue,
# DO_BASEGUARD. When findFun finds another binding first (`length` defined
# globally), the guard reads base's binding with SymbolValue, and an
# unforced promise there is evaluated with eval(sym, R_BaseEnv): one depth
# step more than robigo-rt's base_guard takes (it forces through
# eval::symbol_value). At expressions = 45 GNU R overflows; robigo-rt
# still has the level. With base's binding a value both need 45 (no step).
# The closures, compiled by GNU R 4.6.2's compiler::cmpfun and unserialized
# here (GNU R's serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     f = compiler::cmpfun(function(x) length(x)),
#     rec = compiler::cmpfun(function(d) if (d) rec(d - 1L) else f(1:3)))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below). The overflow ends the script: tryCatch
# would read base's `length` itself.
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n5\nUTF-8\n531\n2\n1027\n253\n1026\n1\n262153\n1\nx\n2", 
  "51\n254\n21\n1\n13\n12\n12\n123\n0\n11\n26\n1\n20\n2\n33\n39\n0\n1\n4\n6\n254\n0\n", 
  "1\n262153\n6\nlength\n2\n254\n0\n511\n0\n254\n1\n767\n1\n511\n13\n781\n12\nNA", 
  "\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n1026\n1\n262153\n5\nclass\n16\n1\n262153\n16\n", 
  "expressionsIndex\n254\n1027\n253\n1026\n1\n262153\n1\nd\n251\n254\n21\n5", 
  "\n13\n20\n12\n20\n1\n3\n0\n13\n23\n2\n29\n4\n38\n3\n1\n23\n5\n29\n7\n38\n6\n1\n9\n6\n", 
  "254\n0\n1\n262153\n2\nif\n2\n254\n0\n1279\n2\n254\n244\n0\n6\n254\n0\n1\n26215", 
  "3\n3\nrec\n2\n254\n244\n1\n6\n254\n0\n1\n262153\n1\n-\n2\n254\n0\n1279\n2\n254\n", 
  "0\n13\n1\n1\n0\n254\n0\n254\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n1\nf\n2\n254\n", 
  "244\n3\n6\n254\n0\n1\n262153\n1\n:\n2\n254\n0\n14\n1\n1\n2\n254\n0\n14\n1\n3\n0\n2", 
  "54\n0\n254\n0\n254\n1\n1279\n1\n1791\n243\n0\n21\n13\n8\n12\n20\n1\n16\n2\n45\n0", 
  "\n1\n4\n243\n1\n1\n1279\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n1", 
  "023\n16\n1\n262153\n16\nexpressionsIndex\n254\n1\n2303\n243\n2\n21\n13\n4", 
  "\n12\n16\n1\n1\n4\n243\n3\n13\n238\n2\n1\n262153\n14\ncompact_intseq\n2\n1\n2", 
  "62153\n4\nbase\n2\n13\n1\n13\n254\n14\n3\n3\n1\n1\n254\n243\n2\n13\n781\n4\nNA\n", 
  "2\n2\n2\n1026\n1023\n16\n1\n262153\n16\nexpressionsIndex\n254\n13\n781\n2", 
  "0\nNA\n1\n1\n0\n0\n0\n3\n3\n3\n3\n3\n3\n3\n6\n6\n6\n6\n6\n6\n6\n1026\n1023\n16\n1\n26", 
  "2153\n16\nexpressionsIndex\n254\n1026\n1\n262153\n5\nnames\n16\n2\n2621", 
  "53\n1\nf\n262153\n3\nrec\n254\n"), collapse = "")))
f <- cl$f
rec <- cl$rec
e <- baseenv()
unlockBinding("length", e)
delayedAssign("length", .Primitive("length"), assign.env = e)
length <- function(x) "shadow"
options(expressions = 45L)
r <- rec(40L)
cat("reached", r, "\n")
