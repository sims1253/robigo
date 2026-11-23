# tryCmpfun (#964): a top-level loop whose compile an error ends
# (tryCompile) runs uncompiled. Run under LC_ALL=C.
options(warn = 2)
r <- try(for (i in 1:2) { "\u4e2d" <- i }, silent = TRUE); cat(r)
options(warn = 1)
for (i in 1:2) { "\u4e2d" <- i }
