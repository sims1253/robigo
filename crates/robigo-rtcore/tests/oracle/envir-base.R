# Base bindings list in GNU R's symbol-table walk: chain R_Newhashpjw(name) %% 49157
# ascending, newest install first within a chain. The names below become symbols
# only through the makeActiveBinding() calls, in this order (base is locked;
# R_MakeActiveBinding's base branch does not check the frame lock).
nm <- c("zebra_q", "nqz9500", "apple_q", "nqz104", "mango_q")
for (n in nm) makeActiveBinding(n, function() 1, baseenv())
l <- ls(baseenv(), sorted = FALSE)
cat("base.order :", paste(l[l %in% nm], collapse = " "), "\n")
