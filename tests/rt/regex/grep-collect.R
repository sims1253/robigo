# strsplit(), gsub()/sub(), gregexpr(), regexec(), and regexpr() long enough
# to collect inside one call (Cx::fill_list / Cx::fill_strings) under
# gctorture, young-moving GC, and retry-torture.

# strsplit(): fixed, ERE, PCRE, split = "", NA, UTF-8, named input, multi-token
x1 <- paste0("a,", 1:400)
names(x1) <- paste0("n", 1:400)
s1 <- strsplit(x1, ",", fixed = TRUE)
cat("sp-fixed:", length(s1), s1[[1]], s1[[400]], names(s1)[1], names(s1)[400], "\n")

x2 <- rep(c("a1b22", "é3ö", NA_character_), length.out = 240)
s2 <- strsplit(x2, "[0-9]+")
s3 <- strsplit(x2, "\\d+", perl = TRUE)
s4 <- strsplit(rep(c("abc", "été", NA_character_), length.out = 150), "")
s5 <- strsplit(rep(c("a-b", "c_d"), 100), c("-", "_"))
cat("sp-re:", length(s2), s2[[1]], s2[[2]], is.na(s2[[3]]),
    identical(s2, s3), length(s4[[2]]), s4[[2]], s5[[1]], s5[[2]], "\n")

# gsub() / sub(): fixed, ERE, PCRE with backrefs, UTF-8, NA, attributes
gx <- structure(paste0("café_", 1:400, "_end"), names = paste0("g", 1:400), class = "mycls")
gx[200] <- NA_character_
g1 <- gsub("_", "-", gx, fixed = TRUE)
g2 <- gsub("([0-9]+)", "[\\1]", gx)
g3 <- sub("([a-z]+)_(\\d+)", "\\U\\1\\E:\\2", gx, perl = TRUE)
cat("gsub:", length(g1), g1[1], is.na(g1[200]), g1[400], class(g1), names(g1)[400], "\n")
cat("gsub-re:", g2[1], g2[400], g3[1], g3[400], "\n")

# gregexpr(): fixed, ERE, PCRE with named capture groups, NA
tx <- rep(c("ab12cd34", "éa1éa2", NA_character_, "none"), length.out = 200)
ge1 <- gregexpr("a", tx, fixed = TRUE)
ge2 <- gregexpr("[0-9]+", tx)
ge3 <- gregexpr("(?<let>[a-z])(?<num>\\d)", tx, perl = TRUE)
cat("greg-fixed:", length(ge1), ge1[[1]], attr(ge1[[1]], "match.length"), is.na(ge1[[3]]), ge1[[4]], "\n")
cat("greg-ere:", length(ge2), ge2[[1]], attr(ge2[[1]], "match.length"), ge2[[2]], "\n")
cat("greg-pcre:", length(ge3), ge3[[1]], attr(ge3[[1]], "capture.names"),
    attr(ge3[[1]], "capture.start"), attr(ge3[[1]], "capture.length"), "\n")

# regexec(): ERE with capture groups, fixed, NA
rx1 <- regexec("([a-z]+)([0-9]+)", tx)
rx2 <- regexec("éa", tx, fixed = TRUE)
cat("regexec:", length(rx1), rx1[[1]], attr(rx1[[1]], "match.length"),
    is.na(rx1[[3]]), rx1[[4]], rx2[[2]], "\n")

# regexpr(): PCRE with capture groups
re1 <- regexpr("(?<let>[a-z]+)(?<num>\\d+)", tx, perl = TRUE)
cat("regexpr:", length(re1), re1[1], re1[2], is.na(re1[3]), re1[4],
    attr(re1, "capture.names"), attr(re1, "capture.start")[1, ], "\n")
