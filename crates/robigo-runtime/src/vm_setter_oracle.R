# Record with GNU R 4.5.0-3 and R_ENABLE_JIT=0.
# Rscript --vanilla crates/robigo-runtime/src/vm_setter_oracle.R > crates/robigo-runtime/src/vm_setter_oracle.txt
for (src in c('m[2, ] <- m[1, ]', 'm[2, ] <- 7', 'm[(n <- n + 1L) + 1L, 1] <- m[1, 1]', 'm[NA_real_] <- 1:2', 'm[2, 1] <- m[1, 1]', 'm[[2, 1]] <- m[1, 1]', 'm[2, ] <- { n <- n + 1L; m[1, ] }')) {
  for (mode in c('ast', 'vm')) {
    f <- eval(parse(text=paste('function(m) { n <- 0L; ', src, '}')))
    if (mode == 'vm') f <- compiler::cmpfun(f)
    tryCatch(f(matrix(0, 1, 1)), error=function(e) cat(mode, src, paste(deparse(conditionCall(e)), collapse=' '), conditionMessage(e), sep='\t'))
    cat('\n')
  }
}
