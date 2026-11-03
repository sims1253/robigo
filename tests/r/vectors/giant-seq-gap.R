## GNU R's compact sequences are ALTREP: 1:-2147483648 stays virtual, so
## asking its length needs no memory. robigo materialises compact sequences;
## its allocation probe stops the 16 Gb request with R's own "cannot
## allocate" error. Documents that gap (see docs/ledger/conformance-gnur-tests.md).
s <- 1:-2147483648L
print(length(s))
print(typeof(s))
