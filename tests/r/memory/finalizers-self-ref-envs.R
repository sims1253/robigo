# Self-referencing environments, each with many finalizers, dropped in a
# loop (found by scripts/fuzz-eval.py --tiers): GNU R finishes; robigo dies
# of SIGSEGV in every tier (one environment with 1.2M finalizers, or the
# same environments without the self-reference, do not crash).
for (i in 1:2000) {
    e <- new.env()
    e$self <- e
    for (j in 1:300) reg.finalizer(e, function(e) NULL)
}
cat("done\n")
