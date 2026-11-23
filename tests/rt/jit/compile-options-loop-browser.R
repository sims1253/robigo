# A top-level loop cmp.R leaves uncompiled because it may call browser()
# (compile's mayCallBrowser): its function literal is never compiled, so its
# compilerOptions are not read (only the compile's entry reads them).
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
for (i in 1:2) { if (i < 0) browser(); g <- function() NULL }
print(i)
