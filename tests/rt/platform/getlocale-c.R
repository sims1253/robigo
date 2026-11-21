# Sys.getlocale under LC_ALL=C (getlocale-c.locale): every category "C",
# so LC_ALL is the one name; a change to C.UTF-8 of a category that is not
# LC_CTYPE or LC_COLLATE makes it a composite.
print(Sys.getlocale())
print(Sys.getlocale("LC_CTYPE"))
print(Sys.setlocale("LC_TIME", "C.UTF-8"))
print(Sys.getlocale())
print(Sys.setlocale("LC_TIME", "C"))
print(Sys.getlocale())
