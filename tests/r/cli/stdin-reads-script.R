# Under Rscript (and R -f) the script is the console: stdin() and
# scan(file = "") read the lines that follow the statement being run.
inp <- readLines(n = 2)
line one
  line two with spaces  
inp
y <- readLines(n = 1); z <- 3
after the semicolon
y
z
scan(n = 2)
4 5
scan(what = "", nlines = 1)
a b c
# Lines that are not R code: bytes invalid in UTF-8.
b <- readLines(stdin(), n = 1)
x«z ça
nchar(b, "bytes")
charToRaw(b)
# Not interactive: readline() answers with an empty line.
readline("Name? ")
n <- scan(nmax = 1, quiet = TRUE)
7
n
