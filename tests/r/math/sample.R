# .Internal(sample()) and .Internal(sample2()): rejection sampling
# (R >= 3.6.0), Walker alias and unequal-probability sampling
set.seed(42)
.Internal(sample(10, 10, FALSE, NULL))
.Internal(sample(10, 5, TRUE, NULL))
.Internal(sample(1e6, 5, FALSE, NULL))
.Internal(sample(3e9, 3, TRUE, NULL))
.Internal(sample(1, 3, TRUE, NULL))
.Internal(sample(5, 1, FALSE, NULL))
.Internal(sample(5, 0, FALSE, NULL))
set.seed(1)
.Internal(sample(5, 10, TRUE, c(0.1, 0.2, 0.3, 0.2, 0.2)))
.Internal(sample(5, 3, FALSE, c(0.1, 0.2, 0.3, 0.2, 0.2)))
.Internal(sample(4, 4, FALSE, c(1, 1, 1, 1)))
.Internal(sample(3, 6, TRUE, c(0, 1, 2)))
# more than 200 non-negligible probabilities: Walker alias method
p <- (1:300) / 300
.Internal(sample(300, 12, TRUE, p))
set.seed(99)
.Internal(sample2(1e8, 5))
.Internal(sample2(10, 5))
.Internal(sample2(5e9, 3))
