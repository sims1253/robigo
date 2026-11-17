# base's compact sequences (`:`, seq_len, seq_along) seen from package C,
# as altclasses.c serves them: LENGTH and the elements without expanding
# (DATAPTR_OR_NULL stays NULL), INTEGER, REAL and DATAPTR_RO expand once,
# and an expanded sequence still serializes as its state.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/compact_seq.so"))
x <- 1:10
print(.Call("rt_cs_info", x))
print(.Call("rt_cs_elts", x, c(0L, 4L, 9L)))
# CHECK_BOUNDS_ELT lets i == XLENGTH(x) through: the class computes it
print(.Call("rt_cs_elts", x, 10L))
print(tryCatch(.Call("rt_cs_elts", x, 11L), error = conditionMessage))
print(tryCatch(.Call("rt_cs_elts", x, -1L), error = conditionMessage))
print(.Call("rt_cs_info", x))
print(.Call("rt_cs_dataptr_sum", x))
print(.Call("rt_cs_info", x))
print(.Call("rt_cs_elts", x, c(0L, 9L)))
# decreasing, from seq_len and seq_along, and a real sequence
y <- 5:-2
print(.Call("rt_cs_info", y))
print(.Call("rt_cs_elts", y, 0:7))
print(.Call("rt_cs_ro_sum", y))
print(.Call("rt_cs_info", y))
print(.Call("rt_cs_info", seq_len(6)))
print(.Call("rt_cs_info", seq_along(letters)))
r <- 2147483647:2147483650
print(.Call("rt_cs_info", r))
# type and attribute queries leave it unexpanded
print(c(typeof(r), class(r), is.numeric(r), is.double(r), is.vector(r), is.null(attributes(r))))
print(.Call("rt_cs_info", r))
print(.Call("rt_cs_elts", r, 0:3))
print(.Call("rt_cs_dataptr_sum", r))
print(.Call("rt_cs_info", r))
# a standard vector
print(.Call("rt_cs_info", c(1L, 2L, 3L)))
print(.Call("rt_cs_elts", c(1L, 2L, 3L), 0:2))
# the type checks
print(tryCatch(.Call("rt_cs_int_elt", c(1, 2), 0L), error = conditionMessage))
print(tryCatch(.Call("rt_cs_int_elt", 1:3 + 0.5, 0L), error = conditionMessage))
print(tryCatch(.Call("rt_cs_dataptr_of", environment()), error = conditionMessage))
print(tryCatch(.Call("rt_cs_dataptr_of", quote(f(x))), error = conditionMessage))
# C writes into the expansion it was handed; R then reads the expansion,
# and serialize still writes the sequence's state
z <- 1:5
invisible(.Call("rt_cs_write", z, 1L, 100L))
print(z)
print(z[2])
print(sum(z))
print(unserialize(serialize(z, NULL)))
