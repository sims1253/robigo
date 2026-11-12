# GNU R's `:`, seq_len and seq() return compact sequences (altclasses.c
# compact_intseq / compact_realseq): only length, first and increment,
# expanded only when C asks for DATAPTR. length(), [i], [[i]], for and
# sum() read them without allocating, so these run in no memory. robigo
# stores every vector expanded (value.rs Vector::data), so each needs
# 800 Gb (issue #460; see also giant-seq-gap.R). A gap until Vector has a
# compact representation.
x <- 1:1e11
length(x)
x[5]
x[[1e11]]
x[c(1, 1e11)]
sum(x)
y <- seq_len(1e11)
length(y)
y[2]
z <- seq(1, 1e11)
length(z)
z[3]
for (i in 1:1e11) if (i > 2) break
i
