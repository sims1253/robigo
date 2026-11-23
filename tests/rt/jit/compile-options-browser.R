# With suppressAll FALSE, code cmp.R leaves uncompiled because it may call
# browser() (mayCallBrowser) prints no note and no compilation failure:
# robigo-rt's not-implemented failure for the notes comes only where the
# code's generation begins, after that check, for a loop and a closure.
compiler::setCompilerOptions(suppressAll = FALSE)
for (i in 1:2) if (i < 0) browser()
print(i)

f <- function(x) { for (i in 1:2) NULL; if (x) browser(); 1 }
print(f(FALSE))
