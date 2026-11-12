# Gap: seek() on a text-mode reading file connection returns the OS file
# position, which includes R's 4096-byte read-ahead buffer (connections.c
# buff_seek -> file_seek returns f_tell(fp)), and a tell (where = NA)
# subtracts the unread part of that buffer. robigo's single read buffer
# reports 4 for both after scan(); R reports 5 and 12.
tmp <- tempfile()
cat(c("1\t2\t3", "4\t5\t6"), file = tmp, sep = "\n")
fd <- file(tmp, open = "rt")
scan(file = fd, what = double(), n = 2)
seek(fd)
seek(con = fd, where = 0, origin = "start")
z <- scan(file = fd, what = double(), n = 2)
z
seek(fd)
close(fd)
unlink(tmp)
