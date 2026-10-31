# unloadNamespace runs the package's .onUnload hook; methods' hook calls
# back into R (.isMethodsDispatchOn) before library.dynam.unload.
loadNamespace("methods")
unloadNamespace("methods")
cat("ok\n")
