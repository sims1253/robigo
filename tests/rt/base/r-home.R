# R.home(): the front end's R_HOME (robigo-rt's tree, GNU R's install).
print(identical(R.home(), Sys.getenv("R_HOME")))
print(nzchar(R.home()))
print(nzchar(Sys.getenv(c("R_SHARE_DIR", "R_INCLUDE_DIR", "R_DOC_DIR"))))
print(tryCatch(.Internal(R.home(1)), error = function(e) conditionMessage(e)))
