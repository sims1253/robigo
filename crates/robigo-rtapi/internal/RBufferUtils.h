/* robigo's RBufferUtils.h for GNU R's base-package natives: GNU R 4.6.2's
 * src/main/RBufferUtils.h, whose functions robigo-rtapi exports (memory.c's,
 * ported). */
#ifndef R_BUFFER_UTILS
#define R_BUFFER_UTILS

#include <stddef.h>

typedef struct {
 char *data;
 size_t bufsize;
 size_t defaultSize;
} R_StringBuffer;

/* Note that R_StringBuffer *buf needs to be initialized before call */
void *R_AllocStringBuffer(size_t blen, R_StringBuffer *buf);
void R_FreeStringBuffer(R_StringBuffer *buf);
void R_FreeStringBufferL(R_StringBuffer *buf);

#endif
