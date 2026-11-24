# L7 (amended 2026-11-24): cells kept alive by data a builtin makes, as the
# difference of two gc() deltas (keeping the object vs making and dropping a
# zero-size counterpart through the same call path, which cancels both the
# measuring call's own cells and any quarantined frames), and memory.profile()
# deltas by type for the same objects.

e <- new.env(parent = emptyenv())
e$x <- NULL

f_num   <- function() numeric(1e5)
f_num0  <- function() { numeric(0); NULL }
f_list  <- function() .Internal(as.vector(1:1000, "list"))
f_list0 <- function() { .Internal(as.vector(1:1, "list")); NULL }
f_int   <- function() integer(1e4)
f_int0  <- function() { integer(0); NULL }

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

measure <- function(f, f0) {
  list(gc = m_gc(f) - m_gc(f0),
       mp = m_mp(f) - m_mp(f0))
}

# Warm up closures, bytecode, and first-use CHARSXPs (including memory.profile's
# type names, kept alive so the CHARSXP cache stays stable across calls).
mp_names <- names(memory.profile())
for (k in 1:2) {
  invisible(measure(f_num, f_num0))
  invisible(measure(f_list, f_list0))
  invisible(measure(f_int, f_int0))
}

r_num  <- measure(f_num, f_num0)
r_list <- measure(f_list, f_list0)
r_int  <- measure(f_int, f_int0)

show_res <- function(label, r) {
  cat(label, "gc:", r$gc, "\n")
  nz <- r$mp[r$mp != 0L]
  cat(label, "mp:", paste(names(nz), nz, sep = "=", collapse = " "), "\n")
}

show_res("numeric_1e5", r_num)
show_res("list_1000", r_list)
show_res("integer_1e4", r_int)
