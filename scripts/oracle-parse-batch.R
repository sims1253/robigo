# Batch version of oracle-parse.R: dump the parse of every file in a manifest.
#
# Usage: Rscript --vanilla scripts/oracle-parse-batch.R MANIFEST OUTFILE
#
# MANIFEST holds one `id \t path \t enc` line per source, where enc is
#   u  the file is UTF-8 (or plain ASCII): parse it as is
#   l  the file is Latin-1: transcode to UTF-8 into a temp file first, so
#      that both GNU R and robigo see the same UTF-8 bytes
# OUTFILE gets one physical line per id:
#   id \t dump \x01 dump \x01 ...            parse succeeded (no lines: empty)
#   id \t ERROR <line> <col> <message>       parse failed
# Dumps never contain raw \n, \t or \x01 bytes (encodeString escapes those),
# so the joined line is unambiguous. High bytes are kept as is; compare
# byte-wise.

main_disabled <- TRUE
oracle <- file.path(dirname(sub("--file=", "", grep("--file=", commandArgs(), value = TRUE))), "oracle-parse.R")
env <- new.env()
sys.source(oracle, envir = env, keep.source = FALSE, toplevel.env = env)

args <- commandArgs(trailingOnly = TRUE)
manifest <- readLines(args[[1L]], warn = FALSE)
out <- file(args[[2L]], "wb")

latin1_temp <- NULL
on.exit(if (!is.null(latin1_temp)) unlink(latin1_temp), add = TRUE)

for (line in manifest) {
    parts <- strsplit(line, "\t", fixed = TRUE)[[1L]]
    id <- parts[[1L]]
    path <- parts[[2L]]
    enc <- if (length(parts) >= 3L) parts[[3L]] else "u"
    if (enc == "l") {
        raw <- readBin(path, "raw", file.info(path)$size)
        s <- if (length(raw)) rawToChar(raw) else ""
        Encoding(s) <- "latin1"
        if (is.null(latin1_temp)) latin1_temp <- tempfile(fileext = ".R")
        writeLines(enc2utf8(s), latin1_temp, useBytes = TRUE)
        path <- latin1_temp
    }
    lines <- tryCatch(
        {
            exprs <- withCallingHandlers(
                parse(file = path, keep.source = FALSE, encoding = "UTF-8"),
                warning = function(w) invokeRestart("muffleWarning")
            )
            vapply(as.list(exprs), env$dump_expr, "")
        },
        error = function(e) env$dump_error(e, path)
    )
    writeLines(paste0(id, "\t", paste(lines, collapse = "\001")), out, useBytes = TRUE)
}
close(out)
cat(length(manifest), "sources dumped\n")
