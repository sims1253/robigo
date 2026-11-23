# R: eval.c:eval (PROMSXP). C code that evaluates a promise with
# eval(x, rho) takes eval's depth step before forcePromise: the promise's
# code runs one level deeper than a bare force. Each line forces a fresh
# promise whose code measures the levels left below the limit.
# promise-eval-depth-volume.R's small twin, for the torture steps (that
# case compiles with compiler::cmpfun, `volume`): the same closures,
# compiled by GNU R 4.6.2's compiler::cmpfun and unserialized here (GNU R's
# serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     g = compiler::cmpfun(function() { N <<- N + 1L; g() }),
#     hr = compiler::cmpfun(function() { N <<- 0L; tryCatch(g(), error = function(e) NULL); N }),
#     k = compiler::cmpfun(function(x, d) if (d > 0L) k(x, d - 1L) else { last <<- environment(); UseMethod("k") }))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below).
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n5\nUTF-8\n531\n3\n1027\n253\n254\n21\n4\n13\n15\n12\n2", 
  "0\n1\n16\n2\n44\n3\n95\n1\n4\n23\n5\n38\n6\n1\n8\n6\n254\n0\n1\n262153\n1\n{\n2\n25", 
  "4\n244\n0\n6\n254\n0\n1\n262153\n3\n<<-\n2\n254\n0\n1\n262153\n1\nN\n2\n254\n24", 
  "4\n1\n6\n254\n0\n1\n262153\n1\n+\n2\n254\n0\n1023\n2\n254\n0\n13\n1\n1\n0\n254\n0", 
  "\n254\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n1\ng\n0\n254\n0\n254\n1\n1023\n13\n", 
  "13\n1\n1\n243\n1\n243\n0\n1\n1535\n243\n2\n13\n781\n15\nNA\n1\n1\n2\n2\n3\n3\n4\n4", 
  "\n4\n6\n6\n6\n6\n6\n1026\n1\n262153\n5\nclass\n16\n1\n262153\n16\nexpression", 
  "sIndex\n254\n1027\n253\n254\n21\n5\n13\n20\n12\n16\n1\n95\n2\n4\n23\n4\n29\n6\n", 
  "29\n7\n31\n8\n38\n5\n4\n20\n2\n1\n10\n6\n254\n0\n511\n2\n254\n244\n0\n6\n254\n0\n7", 
  "67\n2\n254\n0\n1023\n2\n254\n0\n13\n1\n0\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1\n2", 
  "62153\n8\ntryCatch\n2\n254\n244\n2\n6\n254\n0\n1535\n0\n254\n2\n1\n262153\n5", 
  "\nerror\n244\n3\n6\n254\n0\n1\n262153\n8\nfunction\n2\n254\n2\n1\n262153\n1\n", 
  "e\n0\n251\n0\n254\n2\n254\n0\n254\n2\n254\n0\n254\n0\n254\n0\n254\n2\n254\n0\n10", 
  "23\n0\n254\n13\n13\n1\n0\n1\n1023\n243\n0\n1\n2047\n243\n1\n21\n13\n6\n12\n23\n1", 
  "\n38\n0\n1\n3\n243\n2\n1\n1535\n13\n781\n6\nNA\n0\n0\n0\n0\n0\n1026\n1791\n16\n1\n", 
  "262153\n16\nexpressionsIndex\n254\n21\n13\n4\n12\n41\n1\n1\n3\n243\n3\n19\n", 
  "19\n3\n1026\n2815\n251\n254\n21\n1\n13\n3\n12\n17\n1\n3\n0\n254\n6\n254\n0\n255", 
  "9\n2\n254\n2\n2815\n0\n251\n0\n254\n2\n254\n0\n254\n2\n254\n0\n254\n0\n254\n13\n", 
  "781\n3\nNA\n1\n1\n1026\n1791\n16\n1\n262153\n16\nexpressionsIndex\n254\n2", 
  "54\n13\n781\n4\nNA\n0\n0\n0\n1026\n1791\n16\n1\n262153\n16\nexpressionsInd", 
  "ex\n254\n1\n2303\n13\n781\n20\nNA\n1\n1\n3\n3\n3\n5\n5\n5\n5\n5\n5\n5\n5\n5\n5\n5\n2", 
  "\n2\n2\n1026\n1791\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n", 
  "1026\n1\n262153\n1\nx\n251\n1026\n1\n262153\n1\nd\n251\n254\n21\n7\n13\n32\n1", 
  "2\n20\n1\n16\n2\n56\n3\n3\n0\n19\n23\n4\n29\n6\n29\n7\n38\n5\n1\n23\n8\n38\n9\n95\n1", 
  "0\n4\n123\n12\n31\n40\n12\n1\n14\n6\n254\n0\n1\n262153\n2\nif\n2\n254\n244\n0\n6", 
  "\n254\n0\n1\n262153\n1\n>\n2\n254\n0\n3327\n2\n254\n0\n13\n1\n0\n0\n254\n2\n254\n", 
  "244\n1\n6\n254\n0\n1\n262153\n1\nk\n2\n254\n0\n3071\n2\n254\n244\n2\n6\n254\n0\n", 
  "1\n262153\n1\n-\n2\n254\n0\n3327\n2\n254\n0\n13\n1\n1\n0\n254\n0\n254\n2\n254\n6", 
  "\n254\n0\n511\n2\n254\n244\n3\n6\n254\n0\n767\n2\n254\n0\n1\n262153\n4\nlast\n2", 
  "\n254\n244\n4\n6\n254\n0\n1\n262153\n11\nenvironment\n0\n254\n0\n254\n2\n254", 
  "\n244\n5\n6\n254\n0\n1\n262153\n9\nUseMethod\n2\n254\n0\n16\n1\n262153\n1\nk\n", 
  "0\n254\n0\n254\n0\n254\n1\n3327\n13\n13\n1\n0\n243\n0\n1\n4095\n243\n1\n21\n13\n", 
  "4\n12\n20\n0\n1\n3\n1\n3071\n243\n1\n13\n781\n4\nNA\n1\n1\n1\n1026\n1791\n16\n1\n", 
  "262153\n16\nexpressionsIndex\n254\n21\n13\n8\n12\n20\n1\n16\n2\n45\n0\n1\n4", 
  "\n243\n2\n1\n3327\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n1791\n", 
  "16\n1\n262153\n16\nexpressionsIndex\n254\n1\n4863\n243\n4\n1\n4607\n243\n", 
  "3\n243\n5\n13\n781\n32\nNA\n1\n1\n2\n2\n3\n3\n0\n0\n0\n5\n5\n5\n5\n5\n5\n5\n5\n5\n9\n9", 
  "\n9\n9\n11\n11\n11\n12\n12\n12\n12\n12\n12\n1026\n1791\n16\n1\n262153\n16\nexp", 
  "ressionsIndex\n254\n1026\n1\n262153\n5\nnames\n16\n3\n262153\n1\ng\n2621", 
  "53\n2\nhr\n262153\n1\nk\n254\n"), collapse = "")))
g <- cl$g
hr <- cl$hr
k <- cl$k
N <- 0L
e <- environment()
mk <- function() delayedAssign("p", hr(), assign.env = e)
options(expressions = 200L)
cat("direct:            ", hr(), "\n")
mk(); cat("symbol:            ", p, "\n")
mk(); cat("e$p:               ", e$p, "\n")
mk(); cat("e[[\"p\"]]:          ", e[["p"]], "\n")
mk(); cat("get(\"p\"):          ", get("p"), "\n")
mk(); cat("get0(\"p\"):         ", get0("p"), "\n")
mk(); cat("mget(\"p\"):         ", mget("p", envir = e)[[1L]], "\n")
f <- new.env()
mkf <- function() delayedAssign("q", hr(), assign.env = f)
mkf(); cat("as.list(f):        ", as.list(f)$q, "\n")
mkf(); cat("eapply(f, I):      ", eapply(f, function(v) v)$q, "\n")
gen <- function(x) UseMethod("gen")
gen.default <- function(x) x
cat("UseMethod (fresh): ", gen(hr()), "\n")
gen2 <- function(x) { x; UseMethod("gen") }
cat("UseMethod (forced):", gen2(hr()), "\n")
options(expressions = 5000L)

# a promise interrupted at the limit by UseMethod's GetObject restarts once
k.default <- function(x, d) x
last <- NULL
options(expressions = 25L)
r1 <- tryCatch(k(42L + 0L, 13L), error = function(x) conditionMessage(x))
options(expressions = 5000L)
W <- 0L
r2 <- tryCatch(withCallingHandlers(last$x, warning = function(w) {
    W <<- W + 1L
    invokeRestart("muffleWarning")
}), error = function(x) conditionMessage(x))
print(r1)
print(r2)
cat("restart warnings:", W, "\n")
