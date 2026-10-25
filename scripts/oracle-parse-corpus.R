# Batch version of oracle-parse.R: dump every *.R file under a directory.
# Usage: Rscript --vanilla scripts/oracle-parse-corpus.R SRC_DIR OUT_DIR
args <- commandArgs(trailingOnly = TRUE)
src <- normalizePath(args[[1L]])
out <- args[[2L]]
oracle <- file.path(dirname(sub("--file=", "", grep("--file=", commandArgs(), value = TRUE))), "oracle-parse.R")
env <- new.env()
sys.source(oracle, envir = env, keep.source = FALSE, toplevel.env = env)
files <- list.files(src, pattern = "\\.[R]$", recursive = TRUE)
for (f in files) {
  dest <- file.path(out, paste0(f, ".dump"))
  dir.create(dirname(dest), recursive = TRUE, showWarnings = FALSE)
  env$main(c(file.path(src, f), dest))
}
cat(length(files), "files\n")
