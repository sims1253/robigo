# `:`, seq_len, seq_along, seq.int, sequence
1:5
5:1
x <- 0:0
x
typeof(1:3)
typeof(1.5:3)
1.5:4
-2.5:1
2:2.9
typeof(2:2.9)
1e10:(1e10 + 2)
typeof(1e10:(1e10 + 2))
-1:1
TRUE:3
"2":4
c(3, 9):5
seq_len(0)
seq_len(3)
seq_len(2.9)
typeof(seq_len(3))
seq_along(c("a", "b", "c"))
seq_along(NULL)
seq_along(list(1, 2))
seq.int(5)
seq.int(2.5)
seq.int(c(10, 20, 30))
seq.int(0)
seq.int(2, 8)
seq.int(2, 8, by = 2)
seq.int(2L, 8L, by = 2L)
typeof(seq.int(2L, 8L, by = 2L))
typeof(seq.int(2, 8, by = 2))
seq.int(10, 1, by = -3)
seq.int(0, 1, by = 0.1)
seq.int(1, 2, by = 0.3)
seq.int(0, 1, length.out = 4)
seq.int(1, 10, length.out = 4)
typeof(seq.int(1, 10, length.out = 4))
seq.int(5, length.out = 3)
seq.int(to = 5, length.out = 3)
seq.int(to = 5, by = 2, length.out = 3)
seq.int(from = 2, by = 0.5, length.out = 4)
seq.int(along.with = c("x", "y", "z"))
seq.int(3, along.with = 1:4)
seq.int(1, 1)
seq.int(5, 5, by = 2)
seq.int(0, 0, by = 1)
seq.int(length.out = 0)
seq.int(1, 2, length.out = 1)
seq.int(-1, 1, length.out = 5)
sequence(c(3, 2))
sequence(c(2, 0, 3), from = 5)
sequence(3, by = -1)
fx <- c(1L, 2L, NA); levels(fx) <- c("a", "b"); class(fx) <- "factor"
fy <- c(2L, 1L, 1L); levels(fy) <- c("u", "v", "w"); class(fy) <- "factor"
unclass(fx:fy)
seq_len(c(2, 5))
