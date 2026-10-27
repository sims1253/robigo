library(tools)
### Name: userdir
### Title: R User Directories
### Aliases: R_user_dir

### ** Examples
## IGNORE_RDIFF_BEGIN
R_user_dir("FOO", "cache")

## Create one, platform agnostically, must work if <normal> :
(Rdb <- R_user_dir("base"))
if(newD <- !dir.exists(Rdb)) # should work user specifically:
    newD <- dir.create(Rdb, recursive=TRUE)
dir(Rdb) # typically empty
if(newD) unlink(Rdb) # cleaning up

list.files(R_user_dir("grid"), full.names = TRUE)
## IGNORE_RDIFF_END


