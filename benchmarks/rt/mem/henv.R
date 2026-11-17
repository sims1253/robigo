# Memory benchmark (G3 D13, docs/ledger/g3/2026-11-17-m2m3-bars.md): 1e5
# names assigned into one hashed environment (its chains are LISTSXP nodes).
# scripts/rss.py.
e <- new.env(hash = TRUE)
for (i in 1:1e5) assign(paste0("v", i), i, envir = e)
cat(length(ls(e)), "\n")
