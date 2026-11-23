# R: eval.c:setup_bcframe_prom. Byte code's GETVAR (and DDVAL) forces an
# unforced promise of byte code in place: PRSEEN 1 and the depth step come
# before the promise is pending, so an overflow at that step leaves it
# PRSEEN 1 ("promise already under evaluation" later), while the promises
# above it are marked interrupted (the restart warning later). Two chains:
# argument promises, and `...` promises read by a compiled ..1.
# promise-setup-overflow-volume.R's small twin, for the torture steps:
# the same closures, compiled by GNU R 4.6.2's compiler::cmpfun and
# unserialized here (GNU R's serialize(cl, NULL, ascii = TRUE) of the list
#   cl <- list(
#     k = compiler::cmpfun(function(n, d) if (d == 0L) environment() else k(n + 1L, d - 1L)),
#     kd = compiler::cmpfun(function(..., d) if (d == 0L) environment() else kd(..1 + 1L, d = d - 1L)),
#     gd = compiler::cmpfun(function() ..1))
# cut into 60-character pieces, by GNU R 4.6.2 with R_DEFAULT_PACKAGES=NULL:
#   s <- rawToChar(serialize(cl, NULL, ascii = TRUE)); n <- nchar(s)
#   writeLines(deparse(substring(s, seq(1, n, 60), pmin(seq(60, n + 59, 60), n)),
#                      width.cutoff = 72L))
# and wrapped in the call below).
cl <- unserialize(charToRaw(paste0(
  c("A\n3\n263682\n197888\n5\nUTF-8\n531\n3\n1027\n253\n1026\n1\n262153\n1\nn\n2", 
  "51\n1026\n1\n262153\n1\nd\n251\n254\n21\n6\n13\n24\n12\n20\n1\n16\n2\n51\n3\n3\n", 
  "0\n15\n23\n4\n38\n5\n1\n23\n6\n29\n8\n29\n9\n38\n7\n1\n11\n6\n254\n0\n1\n262153\n2", 
  "\nif\n2\n254\n244\n0\n6\n254\n0\n1\n262153\n2\n==\n2\n254\n0\n767\n2\n254\n0\n13", 
  "\n1\n0\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1\n262153\n11\nenvironment\n0\n254", 
  "\n2\n254\n244\n2\n6\n254\n0\n1\n262153\n1\nk\n2\n254\n244\n3\n6\n254\n0\n1\n2621", 
  "53\n1\n+\n2\n254\n0\n511\n2\n254\n0\n13\n1\n1\n0\n254\n2\n254\n244\n4\n6\n254\n0\n", 
  "1\n262153\n1\n-\n2\n254\n0\n767\n2\n254\n0\n13\n1\n1\n0\n254\n0\n254\n0\n254\n1\n", 
  "767\n13\n13\n1\n0\n243\n0\n1\n1535\n243\n1\n1\n1791\n243\n2\n21\n13\n8\n12\n20\n", 
  "1\n16\n2\n44\n0\n1\n4\n243\n3\n1\n511\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n", 
  "0\n0\n1026\n1\n262153\n5\nclass\n16\n1\n262153\n16\nexpressionsIndex\n25", 
  "4\n21\n13\n8\n12\n20\n1\n16\n2\n45\n0\n1\n4\n243\n4\n1\n767\n13\n13\n1\n1\n13\n781", 
  "\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n262153\n16\nexpressionsInde", 
  "x\n254\n13\n781\n24\nNA\n1\n1\n2\n2\n3\n3\n0\n0\n0\n5\n5\n5\n5\n5\n7\n7\n7\n7\n7\n7\n7", 
  "\n7\n7\n1026\n2559\n16\n1\n262153\n16\nexpressionsIndex\n254\n1027\n253\n", 
  "1026\n1\n262153\n3\n...\n251\n1026\n767\n251\n254\n21\n6\n13\n26\n12\n20\n1\n", 
  "16\n2\n51\n3\n3\n0\n15\n23\n4\n38\n5\n1\n23\n6\n29\n8\n29\n9\n31\n1\n38\n7\n1\n11\n6", 
  "\n254\n0\n1023\n2\n254\n244\n0\n6\n254\n0\n1279\n2\n254\n0\n767\n2\n254\n0\n13\n", 
  "1\n0\n0\n254\n2\n254\n244\n1\n6\n254\n0\n1535\n0\n254\n2\n254\n244\n2\n6\n254\n0", 
  "\n1\n262153\n2\nkd\n2\n254\n244\n3\n6\n254\n0\n2047\n2\n254\n0\n1\n262153\n3\n.", 
  ".1\n2\n254\n0\n13\n1\n1\n0\n254\n2\n767\n244\n4\n6\n254\n0\n2303\n2\n254\n0\n767", 
  "\n2\n254\n0\n13\n1\n1\n0\n254\n0\n254\n0\n254\n1\n767\n13\n13\n1\n0\n243\n0\n1\n15", 
  "35\n243\n1\n1\n3071\n243\n2\n21\n13\n8\n12\n21\n1\n16\n2\n44\n0\n1\n4\n243\n3\n1\n", 
  "3327\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n2621", 
  "53\n16\nexpressionsIndex\n254\n21\n13\n8\n12\n20\n1\n16\n2\n45\n0\n1\n4\n243", 
  "\n4\n1\n767\n13\n13\n1\n1\n13\n781\n8\nNA\n1\n1\n2\n2\n0\n0\n0\n1026\n2559\n16\n1\n", 
  "262153\n16\nexpressionsIndex\n254\n13\n781\n26\nNA\n1\n1\n2\n2\n3\n3\n0\n0\n", 
  "0\n5\n5\n5\n5\n5\n7\n7\n7\n7\n7\n7\n7\n7\n7\n7\n7\n1026\n2559\n16\n1\n262153\n16\ne", 
  "xpressionsIndex\n254\n1027\n253\n254\n21\n1\n13\n4\n12\n21\n0\n1\n2\n1\n332", 
  "7\n13\n781\n4\nNA\n0\n0\n0\n1026\n2559\n16\n1\n262153\n16\nexpressionsInde", 
  "x\n254\n1026\n1\n262153\n5\nnames\n16\n3\n262153\n1\nk\n262153\n2\nkd\n2621", 
  "53\n2\ngd\n254\n"), collapse = "")))
k <- cl$k
kd <- cl$kd
gd <- cl$gd
probe <- function(label, mk, force) {
    mk
    force
    L <- 25L
    repeat {
        e <- mk()
        options(expressions = L)
        r <- tryCatch(force(e), error = function(x) "E")
        options(expressions = 5000L)
        if (!identical(r, "E") || L >= 200L) break
        L <- L + 1L
    }
    e <- mk()
    options(expressions = L - 3L)
    r1 <- tryCatch(force(e), error = function(x) conditionMessage(x))
    options(expressions = 5000L)
    W <- 0L
    r2 <- tryCatch(withCallingHandlers(force(e), warning = function(w) {
        W <<- W + 1L
        invokeRestart("muffleWarning")
    }), error = function(x) conditionMessage(x))
    cat(label, "\n")
    print(r1)
    print(r2)
    cat("restart warnings:", W, "\n")
}
probe("argument promises, read by $", function() k(1L, 20L), function(e) e$n)
probe("... promises, read by ..1", function() kd(1L, d = 20L), function(e) {
    environment(gd) <- e
    gd()
})
