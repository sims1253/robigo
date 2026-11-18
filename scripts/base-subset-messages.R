# base-subset.R's messages, apart so that scripts/test-base-subset-messages.R
# can check them without a run.

# The failure when tracing changes what an input prints. Not a stale list
# (regenerating cannot fix it): the case prints something the tracer
# changes. Shows the first line that differs, not the whole transcripts.
traced_differs <- function(file, part, a, b, saved) {
  # (a final newline is a line of its own here: strsplit drops a trailing
  # empty field, and "a" against "a\n" must still show where they differ)
  lines_of <- function(x) {
    l <- strsplit(x, "\n", fixed = TRUE)[[1L]]
    if (endsWith(x, "\n")) c(l, "") else l
  }
  la <- lines_of(a)
  lb <- lines_of(b)
  n <- max(length(la), length(lb))
  i <- which(vapply(seq_len(n), function(k) !identical(la[k], lb[k]), NA))[1L]
  show <- function(x) if (is.na(x)) "(no line)" else if (!nzchar(x)) "(an empty line: a final newline)" else x
  where <- if (is.na(i)) "differs (in no line: compare the transcripts)" else sprintf("differs at line %d", i)
  paste0(
    sprintf("base-subset: %s prints differently when traced: its %s %s.\n", file,
            c(out = "stdout", err = "stderr", status = "exit status")[[part]], where),
    if (!is.na(i)) sprintf("  untraced: %s\n  traced:   %s\n", show(la[i]), show(lb[i])) else "",
    "This is not a stale list, and regenerating cannot fix it: the case prints something that\n",
    "scripts/base-subset-trace.R changes: it gives every closure of base (and of the script) a body\n",
    "that records what runs, so a deparsed function or body, a binding's type, sys.function(), or\n",
    "the protect stack's depth differ.\n",
    "Make the case print what does not depend on that, or leave the case out of the inputs in\n",
    "scripts/base-subset.R and say why there.\n",
    sprintf("Both transcripts: %s", saved))
}
