# C compares CHARSXPs by pointer: in GNU R every CHARSXP is cached, so a
# string's STRING_ELT is mkChar's object for its bytes, its symbol's
# PRINTNAME, and an equal string's elsewhere. Strings made by paste,
# sprintf and coercion are checked from C.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/strings.so"))
p <- paste0("s", 1:3)
print(.Call("rt_elt_is_mkchar", p, 1L))
print(.Call("rt_elt_is_printname", p, 2L))
q <- sprintf("s%d", 3:1)
print(.Call("rt_elts_same", p, 0L, q, 2L))
n <- as.character(c(7, 8))
print(.Call("rt_elt_is_mkchar", n, 0L))
print(.Call("rt_elts_same", n, 1L, as.character(8), 0L))
u <- paste0("é", 1:2)
print(.Call("rt_elts_same", u, 0L, sprintf("é%d", 1L), 0L))
