# printVector for doubles: formatReal's common fixed or scientific
# format, 7 digits, negative zero, the non-finite values, wrapping.
(1:30)/7
-0
1/-Inf
c0 <- 0
-c0
(1:12) * 1e5
(1:12) / 1e5
1e15 + 0:3
2^(1:40)
1/0; -1/0; 0/0; NA_real_
(1:5) + NA
1e-300 * (1:3)
