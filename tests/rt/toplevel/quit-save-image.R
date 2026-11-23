# quit("yes") saves the workspace (R_SaveGlobalEnv, the image dirty:
# R_DirtyImage) as ".RData" in the working directory, through base's
# sys.save.image and saveToConn; an exit finalizer, which runs after the
# save, reads it back. (In a directory of its own: the case's cwd is the
# checkout's.)
d <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-quit-save-", Sys.getpid())
dir.create(d)
setwd(d)
x <- 1:3
y <- list(a = "b")
.Last <- function() cat("last\n")
reg.finalizer(environment(), function(e) {
    cat("saved:", file.exists(".RData"), "\n")
    e <- new.env()
    print(sort(load(".RData", envir = e)))
    print(e$x)
    print(e$y)
    print(e$d == d)
    print(ls(e, all.names = TRUE, sorted = TRUE))
    print(file.remove(".RData"))
    setwd("/")
    print(file.remove(d))
}, onexit = TRUE)
quit("yes")
cat("not reached\n")
