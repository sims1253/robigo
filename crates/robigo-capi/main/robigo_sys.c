/* robigo: GNU R 4.5.0's sysutils.c file and process helpers (Unix
   branches) and connections.c's gzio.h (R's own gzip reader, which
   grDevices uses for its font metric files), copied from those files. */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#include <Defn.h>
#include <Rinterface.h>
#include <errno.h>
#ifdef HAVE_SYS_WAIT_H
# include <sys/wait.h>
#endif
#include <unistd.h>

/* R: connections.c, before it includes gzio.h */
#if defined(HAVE_OFF_T) && defined(HAVE_FSEEKO)
# define f_seek fseeko
# define f_tell ftello
# define OFF_T off_t
#else
# define f_seek fseek
# define f_tell ftell
# define OFF_T long
#endif

#include "gzio.h"

/* R: sysutils.c */
#define fixmode(mode) (mode)

FILE *R_fopen(const char *filename, const char *mode)
{
    return(filename ? fopen(filename, fixmode(mode)) : NULL );
}

FILE *R_popen(const char *command, const char *type)
{
    FILE *fp;
    fp = popen(command, type);
    return fp;
}

int R_system(const char *command)
{
    int res;
    res = system(command);
#ifdef HAVE_SYS_WAIT_H
    if (WIFEXITED(res)) res = WEXITSTATUS(res);
#else
    /* assume that this is shifted if a multiple of 256 */
    if ((res % 256) == 0) res = res/256;
#endif
    if (res == -1) {
	/* this means that system() failed badly - it didn't
	   even get to try to run the shell */
	warning(_("system call failed: %s"), strerror(errno));
	/* R system() is documented to return 127 on failure, and a lot of
	   code relies on that - it will misinterpret -1 as success */
	res = 127;
    }
    return res;
}

char * R_tmpnam(const char * prefix, const char * tempdir)
{
    return R_tmpnam2(prefix, tempdir, "");
}

/* returns malloc'd result */
char * R_tmpnam2(const char *prefix, const char *tempdir, const char *fileext)
{
    unsigned int n, pid = getpid();
    char filesep[] = "/";

    if(!prefix) prefix = "";	/* NULL */
    if(!fileext) fileext = "";  /*  "   */

    for (n = 0; n < 100; n++) {
	/* try a random number at the end.  Need at least 6 hex digits */
	int r1 = rand();
#if RAND_MAX > 16777215
# define TMPNAM2_SNPRINTF(BUF, SIZE) \
	snprintf(BUF, SIZE, "%s%s%s%x%x%s", tempdir, filesep, prefix, pid, r1, fileext)
#else
	int r2 = rand();
# define TMPNAM2_SNPRINTF(BUF, SIZE) \
	snprintf(BUF, SIZE, "%s%s%s%x%x%x%s", tempdir, filesep, prefix, pid, r1, r2, fileext)
#endif
	size_t needed = TMPNAM2_SNPRINTF(NULL, 0) + 1;
	if (needed > R_PATH_MAX)
	    error(_("temporary name too long"));
	char *res = (char *) malloc(needed);
	if(!res)
	    error(_("allocation failed in R_tmpnam2"));
	TMPNAM2_SNPRINTF(res, needed);
	if (!R_FileExists(res))
	    return res;
	free(res);
    }
    error(_("cannot find unused tempfile name"));
}

void R_free_tmpnam(char *name)
{
    if (name) free(name);
}

/* R: unix/X11.c, the branch without HAVE_X11: robigo has no X11 module */
attribute_hidden Rboolean R_access_X11(void)
{
    return FALSE;
}

SEXP do_X11(SEXP call, SEXP op, SEXP args, SEXP rho)
{
    error(_("X11 is not available"));
    return R_NilValue;
}

SEXP do_saveplot(SEXP call, SEXP op, SEXP args, SEXP rho)
{
    error(_("X11 is not available"));
    return R_NilValue;
}

Rboolean R_GetX11Image(int d, void *pximage, int *pwidth, int *pheight)
{
    error(_("X11 is not available"));
    return FALSE;
}

SEXP do_bmVersion(void)
{
    SEXP ans = PROTECT(allocVector(STRSXP, 3)),
	nms = PROTECT(allocVector(STRSXP, 3));
    setAttrib(ans, R_NamesSymbol, nms);
    SET_STRING_ELT(nms, 0, mkChar("libpng"));
    SET_STRING_ELT(nms, 1, mkChar("jpeg"));
    SET_STRING_ELT(nms, 2, mkChar("libtiff"));
    UNPROTECT(2);
    return ans;
}
