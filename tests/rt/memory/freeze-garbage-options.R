# The boot heap's freeze: a boot option's cell (in .Options at the freeze)
# given a finalizable environment, then removed (options(x = NULL) unlinks
# the cell): the environment dies at the next gc(), as in GNU R
# (with ROBIGO_GC_FREEZE=1 the freeze is the first collection after the
# boot: this one, so that what follows is stored into frozen objects)
invisible(gc())
e <- new.env()
reg.finalizer(e, function(x) cat("finalized\n"))
options(dvipscmd = e)
invisible(gc())
cat("held by the option\n")
rm(e)
options(dvipscmd = NULL)
1  # (.Last.value no longer the old option)
invisible(gc())
cat("after gc\n")
