library(grDevices)
# colors() and col2rgb(): the ColorDataBase table, the four hex forms,
# palette indices, and every error path (grDevices/src/colors.c).
length(colors())
colors()[1:5]
colors()[655:657]
colors()[c(1, 100, 657)]
identical(colors(), colors())

col2rgb(c("red", "RED", "  r e d ", "white", "cornflowerblue", "NA",
          "transparent", NA_character_), FALSE)
col2rgb(c("#ff0000", "#f00", "#ff000080", "#f008"), TRUE)
col2rgb(c("1", "2.5", "8", "9", "100", "0.5"), FALSE)
col2rgb(1:10, TRUE)
col2rgb(c(a = "blue", b = 4L), TRUE)
col2rgb(c(1.7, 3), FALSE)
col2rgb(c(1L, 8L, 9L, NA_integer_), TRUE)

for (e in list(
  function() col2rgb("no such color", FALSE),
  function() col2rgb(c(-1L, 2L), FALSE),
  function() col2rgb("x", NA),
  function() col2rgb(c("#gg0000"), FALSE),
  function() col2rgb(c("#ff00"), FALSE),
  function() col2rgb(c("1x"), FALSE)
)) print(tryCatch(e(), error = function(x) conditionMessage(x)))
