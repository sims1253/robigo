# R_alloc from package C (memory.c:R_alloc): an empty request (nelem *
# eltsize not above 0) is NULL, as GNU R's is; a block holds what C writes;
# more than R_XLEN_T_MAX bytes is error()'s "cannot allocate memory block".
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/r_alloc.so"))
.Call("rt_r_alloc_null", 0, 8L)
.Call("rt_r_alloc_null", 5, 0L)
.Call("rt_r_alloc_null", 3, -1L)
.Call("rt_r_alloc_null", 1, 1L)
.Call("rt_r_alloc_fill", 9L)
e <- tryCatch(.Call("rt_r_alloc_huge"), error = function(e) e)
conditionMessage(e)
conditionCall(e)
