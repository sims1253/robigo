## as.character() on a double or complex uses the OutDec option: R's
## coerce.c:StringFromReal calls EncodeRealDrop0(..., OutDec) and
## do_ascomplex's EncodeComplex(..., OutDec), reading the same C global
## that EncodeReal0 consults when printing.
op <- options(OutDec = ",")
print(as.character(1.5))
print(as.character(c(1.5, 2, -0.25)))
print(as.character(1.5 + 2.5i))
print(as.character(-1.5 - 2.5i))
print(as.character(1e-5 + 2e-5i))
print(as.character(NA_real_))
print(as.character(NA_complex_))
cat(format(1.5), "\n")
print(paste0(1.5))
print(1.5)
print(1.5 + 2.5i)
options(op)
print(as.character(1.5))
print(as.character(1.5 + 2.5i))
print(1.5)
print(getOption("OutDec"))
