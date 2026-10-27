# One line can hold several statements; the console parses them a chunk
# at a time, up to each ';' or the newline.
1; 2
x <- 3; x; x + 1
invisible(1); 4
paste("a", "b"); "c"
