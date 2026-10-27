Sys.setenv(RR_TEST_VAR = "hello")
Sys.getenv("RR_TEST_VAR")
Sys.getenv(c("RR_TEST_VAR", "RR_UNSET_VAR"), unset = "<none>")
Sys.getenv("RR_UNSET_VAR")
Sys.getenv("RR_UNSET_VAR", unset = NA)
Sys.unsetenv("RR_TEST_VAR")
Sys.getenv("RR_TEST_VAR")
nzchar(Sys.getenv("HOME"))
class(Sys.time())
is.numeric(unclass(Sys.time()))
pt <- proc.time()
class(pt)
names(pt)
length(pt)
Sys.sleep(0.01)
is.character(date())
nchar(date())
R.version$major
R.version$language
R.version.string
R.Version()$nickname
.Machine$integer.max
.Machine$double.eps
.Platform$file.sep
.Platform$OS.type
names(Sys.info())
interactive()
l10n_info()$`UTF-8`
Sys.getlocale("LC_COLLATE")
Sys.getlocale("LC_NUMERIC")
is.character(tempdir())
dirname(tempfile()) == tempdir()
startsWith(basename(tempfile("abc")), "abc")
endsWith(tempfile(fileext = ".csv"), ".csv")
length(tempfile(c("a", "b")))
file.exists(tempdir())
td <- tempfile()
dir.create(td)
old <- setwd(td)
basename(getwd()) == basename(td)
writeLines("x", "local.txt")
file.exists("local.txt")
normalizePath("local.txt") == file.path(normalizePath(td), "local.txt")
setwd(old)
unlink(td, recursive = TRUE)
file.exists(td)
path.expand("~") == Sys.getenv("HOME")
tryCatch(Sys.sleep(-1), error = function(e) conditionMessage(e))
