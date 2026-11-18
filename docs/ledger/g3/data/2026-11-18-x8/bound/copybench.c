#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/mman.h>
static double now(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec+t.tv_nsec*1e-9;}
/* the realized EnlargeVector sequence: n elements of 8 bytes, cap = floor(i*1.05) */
static double run(long n, int mode) {
  long cap = 0, len = 0; double *x = NULL; size_t bytes = 0; double t0 = now();
  for (long i = 1; i <= n; i++) {
    if (i > cap) {
      long nc = (long)(i * 1.05); if (nc < i) nc = i;
      size_t nb = (size_t)nc * 8;
      if (mode == 0) {               /* copy: new memory, copy, fill tail, free old */
        double *y = malloc(nb);
        if (!y) { perror("malloc"); exit(1); }
        memcpy(y, x, (size_t)len * 8);
        for (long k = len; k < nc; k++) y[k] = 0.0;
        free(x); x = y;
      } else {                       /* ideal remap: mmap'd, grown by mremap (pages move) */
        void *p = x ? mremap(x, bytes, nb, MREMAP_MAYMOVE)
                    : mmap(NULL, nb, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
        if (p == MAP_FAILED) { perror(x ? "mremap" : "mmap"); exit(1); }
        x = p;
        for (long k = len; k < nc; k++) x[k] = 0.0;
      }
      cap = nc; bytes = nb;
    }
    x[i - 1] = (double)i; len = i;
  }
  double t = now() - t0;
  volatile double s = x[n-1]; (void)s;
  if (mode == 0) free(x); else munmap(x, bytes);
  return t;
}
int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: %s N\n", argv[0]); return 2; }
  long n = atol(argv[1]); int reps = 11;
  double c[11], r[11];
  for (int k = 0; k < reps; k++) { c[k] = run(n, 0); r[k] = run(n, 1); }
  /* medians */
  for (int a = 0; a < reps; a++) for (int b = a+1; b < reps; b++) { if (c[b]<c[a]){double t=c[a];c[a]=c[b];c[b]=t;} if (r[b]<r[a]){double t=r[a];r[a]=r[b];r[b]=t;} }
  printf("n=%ld copy %.4f s, ideal remap %.4f s, saved %.4f s\n", n, c[reps/2], r[reps/2], c[reps/2]-r[reps/2]);
  return 0;
}
