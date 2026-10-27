RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: winMenus
### Title: User Menus under MS Windows (RGui)
### Aliases: winMenuAdd winMenuAddItem winMenuDel winMenuDelItem
###   winMenuNames winMenuItems
### Keywords: utilities

### ** Examples
## Not run: 
##D winMenuAdd("Testit")
##D winMenuAddItem("Testit", "one", "aaaa")
##D winMenuAddItem("Testit", "two", "bbbb")
##D winMenuAdd("Testit/extras")
##D winMenuAddItem("Testit", "-", "")
##D winMenuAddItem("Testit", "two", "disable")
##D winMenuAddItem("Testit", "three", "cccc")
##D winMenuAddItem("Testit/extras", "one more", "ddd")
##D winMenuAddItem("Testit/extras", "and another", "eee")
##D winMenuAdd("$ConsolePopup/Testit")
##D winMenuAddItem("$ConsolePopup/Testit", "six", "fff")
##D winMenuNames()
##D winMenuItems("Testit")
## End(Not run)


