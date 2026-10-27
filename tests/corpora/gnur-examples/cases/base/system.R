RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: system
### Title: Invoke a System Command
### Aliases: system shell
### Keywords: interface file utilities

### ** Examples

# list all files in the current directory using the -F flag
## Not run: system("ls -F")

# t1 is a character vector, each element giving a line of output from who
# (if the platform has who)
t1 <- try(system("who", intern = TRUE))

try(system("ls fizzlipuzzli", intern = TRUE, ignore.stderr = TRUE))
# zero-length result since file does not exist, and will give warning.



