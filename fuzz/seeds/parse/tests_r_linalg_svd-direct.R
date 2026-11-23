# LAPACK accepts lowercase jobs and column-major output buffers with padding.
# The unused third row must survive every real and complex job.
options(digits=17)
x <- matrix(c(3,1,2,4),2)
for (job in c("S", "N", "A", "O", "s", "n", "a", "o")) {
  print(tryCatch(.Internal(La_svd(job,x,double(2),matrix(0,3,2),matrix(0,3,2))),error=conditionMessage))
  print(tryCatch(.Internal(La_svd_cmplx(job,x+0i,double(2),matrix(0i,3,2),matrix(0i,3,2))),error=conditionMessage))
}
