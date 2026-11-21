# INTERIM(B, datetime.c). Owner: B (the port of do_asPOSIXlt and
# do_formatPOSIXlt).
# Read only by scripts/rt-home-db.sh's three steps that write a `Built:`
# field (tools:::.install_package_description for tools, compiler and
# base; Built is format(Sys.time(), "%Y-%m-%d %H:%M:%S", tz = "UTC",
# usetz = TRUE)), before each step's script. Never read at robigo-rt's
# start and never in a database: the steps that write compiler.rdb and
# base.rdb run without it.
# Stand-ins, UTC only, for what that call needs: as.POSIXlt.POSIXct and
# format.POSIXlt (E's build walk, 2026-11-21).
# It expires by itself: rt-home-db.sh stops, saying to delete this file,
# once the robigo-rt it runs implements .Internal(as.POSIXlt()).
.rt_shim <- function(name, f) {
    e <- baseenv()
    environment(f) <- .BaseNamespaceEnv
    unlockBinding(name, e)
    assign(name, f, envir = e)
    lockBinding(name, e)
}

## as.POSIXlt.POSIXct (B, datetime.c's do_asPOSIXlt): UTC only,
## the broken-down time by days-from-civil arithmetic
.rt_shim("as.POSIXlt.POSIXct", function(x, tz = "", ...) {
    s <- floor(unclass(x)); frac <- unclass(x) - s
    days <- s %/% 86400; secs <- s %% 86400
    z <- days + 719468; era <- z %/% 146097; doe <- z - era * 146097
    yoe <- (doe - doe %/% 1460 + doe %/% 36524 - doe %/% 146096) %/% 365
    y <- yoe + era * 400; doy <- doe - (365 * yoe + yoe %/% 4 - yoe %/% 100)
    mp <- (5 * doy + 2) %/% 153; d <- doy - (153 * mp + 2) %/% 5 + 1
    m <- mp + ifelse(mp < 10, 3, -9); y <- y + (m <= 2)
    leap <- (y %% 4 == 0 & y %% 100 != 0) | y %% 400 == 0
    cum <- c(0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334)
    structure(list(sec = secs %% 60 + frac, min = as.integer((secs %/% 60) %% 60),
                   hour = as.integer(secs %/% 3600), mday = as.integer(d), mon = as.integer(m - 1),
                   year = as.integer(y - 1900), wday = as.integer((days + 4) %% 7),
                   yday = as.integer(cum[m] + d - 1 + (m > 2 & leap)), isdst = 0L,
                   zone = "UTC", gmtoff = 0L),
              class = c("POSIXlt", "POSIXt"), tzone = "UTC", balanced = TRUE)
})

## format.POSIXlt (B, datetime.c's do_formatPOSIXlt, in work): %Y %m %d
## %H %M %S %Z %%, and usetz's " UTC"
.rt_shim("format.POSIXlt", function(x, format = "", usetz = FALSE, digits = getOption("digits.secs"), ...) {
    if (!nzchar(format)) format <- "%Y-%m-%d %H:%M:%S"
    p2 <- function(v) formatC(as.integer(v), width = 2, flag = "0")
    out <- format
    out <- gsub("%Y", as.character(x$year + 1900L), out, fixed = TRUE)
    out <- gsub("%m", p2(x$mon + 1L), out, fixed = TRUE)
    out <- gsub("%d", p2(x$mday), out, fixed = TRUE)
    out <- gsub("%H", p2(x$hour), out, fixed = TRUE)
    out <- gsub("%M", p2(x$min), out, fixed = TRUE)
    out <- gsub("%S", p2(floor(x$sec)), out, fixed = TRUE)
    out <- gsub("%Z", "UTC", out, fixed = TRUE)
    out <- gsub("%%", "%", out, fixed = TRUE)
    if (usetz) out <- paste(out, "UTC")
    out
})
