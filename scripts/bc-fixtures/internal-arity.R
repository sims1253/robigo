# Byte-code translation cases: GNU R's compiled .Internal call with a
# count its checkArity refuses (CALLBUILTIN on the internal). GNU R raises
# the arity error when it runs; robigo-rt refuses to translate such code
# (the VM's register call does not check arity: the from-source compiler
# routes a wrong count through the checked list path, which byte code's
# evaluated frame cannot take). A known difference
# (docs/ledger/g3/2026-11-17-b5a-internal-arity.md), not ratcheted.
cases <- list(
  list(name = "k_getenv0", fun = function() .Internal(Sys.getenv()),
       calls = "print(tryCatch(k_getenv0(), error = conditionMessage))"),
  list(name = "k_paste0_ok", fun = function(x) .Internal(paste0(list(x), NULL)),
       calls = "print(k_paste0_ok('a'))")
)
