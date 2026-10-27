RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: changedFiles
### Title: Detect which Files Have Changed
### Aliases: fileSnapshot changedFiles print.changedFiles
###   print.fileSnapshot
### Keywords: utilities file

### ** Examples

# Create some files in a temporary directory
dir <- tempfile()
dir.create(dir)
writeBin(1L, file.path(dir, "file1"))
writeBin(2L, file.path(dir, "file2"))
dir.create(file.path(dir, "dir"))

# Take a snapshot
snapshot <- fileSnapshot(dir, timestamp = tempfile("timestamp"), md5sum=TRUE)
## Don't show: 
Sys.sleep(.01)
## End(Don't show)
  
# Change one of the files.
writeBin(3L:4L, file.path(dir, "file2"))

# Display the detected changes.  We may or may not see mtime change...
changedFiles(snapshot)
changedFiles(snapshot)$changes



