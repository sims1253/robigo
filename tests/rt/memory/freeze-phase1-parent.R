# The boot heap's freeze: an object made after the boot whose only path from
# the roots goes through a boot environment (Autoloads, attached by the
# system profile). With ROBIGO_GC_FREEZE=1 the first collection freezes
# that environment's frame and leaves the object ordinary: the freeze logs
# the frame as it meets the object, and marking keeps the object alive from
# the log. Nothing else refers to it, so if the log missed it the object
# would be freed and its memory reused by the allocations below.
a <- as.environment("Autoloads")
assign(".freeze_phase1", list(x = c(3.5, 4.5), s = paste0("kept", 1:3)), envir = a)
rm(a)
# (with ROBIGO_GC_FREEZE=1 the freeze is the first collection after the
# boot: this one)
invisible(gc())
for (i in 1:200) junk <- lapply(1:50, function(j) c(i, j, i * j))
invisible(gc())
for (i in 1:200) junk <- paste0("junk", 1:50, i)
invisible(gc())
print(get(".freeze_phase1", envir = as.environment("Autoloads")))
