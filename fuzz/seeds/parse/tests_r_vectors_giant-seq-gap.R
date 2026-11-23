## GNU R's compact sequences are ALTREP: 1:-2147483648 stays virtual, so
## asking its length needs no memory. robigo's compact sequences stay
## unexpanded too (value/compact.rs) until something needs their elements.
s <- 1:-2147483648L
print(length(s))
print(typeof(s))
