# R: util.c:R_strtod5; exact decimal powers and adjacent fallback exponents.
digits <- c("0", "-0", "1", "-1", "9007199254740993",
            "123456789012345678", "123456789012345678901234567890123456789",
            "1.2345678901234567", "-123.45678901234567")
for (exponent in c(-324L, -308L, -307L, -19L:19L, 308L)) {
    text <- paste0(digits, "e", exponent)
    cat(exponent, sprintf("%a", as.numeric(text)), "\n")
}
