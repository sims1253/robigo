# L7 (amended 2026-11-24): cells kept alive by data a builtin makes, as the
# difference of two gc() deltas (cancelling the measuring call's own cells),
# and memory.profile() deltas by type for the same objects.

e <- new.env(parent = emptyenv())
e$x <- NULL

f_null <- function() NULL
f_num  <- function() numeric(1e5)
f_list <- function() .Internal(as.vector(1:1000, "list"))
f_int  <- function() integer(1e4)

m_gc <- function(make) {
  gc()
  g0 <- gc()[, "used"]
  e$x <- make()
  g1 <- gc()[, "used"]
  e$x <- NULL
  g1 - g0
}

m_mp <- function(make) {
  gc()
  p0 <- memory.profile()
  e$x <- make()
  p1 <- memory.profile()
  e$x <- NULL
  p1 - p0
}

measure <- function(f, base_gc, base_mp) {
  list(gc = m_gc(f) - base_gc, mp = m_mp(f) - base_mp)
}

# Warm up closures, bytecode, and first-use CHARSXPs (including memory.profile's
# type names, kept alive so the CHARSXP cache stays stable across calls).
mp_names <- names(memory.profile())
z0 <- c(0, 0)
p_zero <- integer(length(mp_names))
for (k in 1:2) {
  for (f in list(f_null, f_num, f_list, f_int)) {
    invisible(measure(f, z0, p_zero))
  }
}

base_gc <- m_gc(f_null)
base_mp <- m_mp(f_null)

r_num  <- measure(f_num, base_gc, base_mp)
r_list <- measure(f_list, base_gc, base_mp)
r_int  <- measure(f_int, base_gc, base_mp)

show_res <- function(label, r) {
  cat(label, "gc:", r$gc, "\n")
  nz <- r$mp[r$mp != 0L]
  cat(label, "mp:", paste(names(nz), nz, sep = "=", collapse = " "), "\n")
}

show_res("numeric_1e5", r_num)
show_res("list_1000", r_list)
show_res("integer_1e4", r_int)
