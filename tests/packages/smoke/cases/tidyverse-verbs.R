# needs: dplyr tidyr tibble
# The data-mask verbs as a user types them: every expression forces the
# mask's active bindings, which subset the chops by the group id the C
# loop rewrites between evaluations (variable contents crossing the
# bridge inside one .Call).
suppressPackageStartupMessages({library(dplyr); library(tidyr); library(tibble)})
df <- tibble(x = 1:6, g = rep(c("a", "b"), 3), y = c(2.5, 1.5, 3, 0.5, 2, 1))
print(filter(df, x > 3, y > 1))
print(mutate(df, z = x * 2 + y, .by = g))
print(summarise(group_by(df, g), m = mean(x), s = sum(y)))
print(mutate(df, across(c(x, y), function(v) v - min(v))))
print(pivot_longer(df, c(x, y), names_to = "k", values_to = "v"))
print(left_join(df, tibble(g = c("a", "b"), lab = c("A", "B")), by = "g"))
print(arrange(df, desc(y), x))
print(count(df, g, wt = y))
