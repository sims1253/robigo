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

/* R: unix/sys-std.c, the history functions without readline, and
   unix/system.c's hooks pointing at them */
#include <Rinterface.h>

attribute_hidden void Rstd_loadhistory(SEXP call, SEXP op, SEXP args, SEXP env)
{
    SEXP sfile;
    char file[R_PATH_MAX];
    const char *p;

    sfile = CAR(args);
    if (!isString(sfile) || LENGTH(sfile) < 1)
	errorcall(call, _("invalid '%s' argument"), "file");
    p = R_ExpandFileName(translateCharFP(STRING_ELT(sfile, 0)));
    if(strlen(p) > R_PATH_MAX - 1)
	errorcall(call, _("'file' argument is too long"));
    strcpy(file, p);
    errorcall(call, _("no history mechanism available"));
}

attribute_hidden void Rstd_savehistory(SEXP call, SEXP op, SEXP args, SEXP env)
{
    SEXP sfile;
    char file[R_PATH_MAX];
    const char *p;

    sfile = CAR(args);
    if (!isString(sfile) || LENGTH(sfile) < 1)
	errorcall(call, _("invalid '%s' argument"), "file");
    p = R_ExpandFileName(translateCharFP(STRING_ELT(sfile, 0)));
    if(strlen(p) > R_PATH_MAX - 1)
	errorcall(call, _("'file' argument is too long"));
    strcpy(file, p);
    errorcall(call, _("no history available to save"));
}

attribute_hidden void Rstd_addhistory(SEXP call, SEXP op, SEXP args, SEXP env)
{
    SEXP stamp;

    checkArity(op, args);
    stamp = CAR(args);
    if (!isString(stamp))
	errorcall(call, _("invalid timestamp"));
}

void (*ptr_R_loadhistory)(SEXP, SEXP, SEXP, SEXP) = Rstd_loadhistory;
void (*ptr_R_savehistory)(SEXP, SEXP, SEXP, SEXP) = Rstd_savehistory;
void (*ptr_R_addhistory)(SEXP, SEXP, SEXP, SEXP) = Rstd_addhistory;

/* R: unix/sys-std.c:Rstd_ClearerrConsole, system.c:R_ClearerrConsole */
void R_ClearerrConsole(void)
{
    clearerr(stdin);
}

/* R: main.c/platform.c: the locale's MB_CUR_MAX, set when the locale is
   (robigo_locale_init, after setlocale) */
int R_MB_CUR_MAX = 1;

void robigo_locale_init(void)
{
    R_MB_CUR_MAX = (int) MB_CUR_MAX;
}

/* R: library/utils/src/io.c:writetable emits cells and separators with literal %s.
   Inspect a copy: other formats still reach the original formatter once. */
const char *robigo_connection_string_arg(const char *format, va_list ap)
{
    if (strcmp(format, "%s") != 0) return NULL;
    va_list args;
    va_copy(args, ap);
    const char *value = va_arg(args, const char *);
    va_end(args);
    return value;
}

/* R: connections.c:Rconn_printf */
#include <R_ext/Connections.h>
int Rconn_printf(Rconnection con, const char *format, ...)
{
    int res;
    errno = 0;
    va_list(ap);
    va_start(ap, format);
    /* R: library/utils/src/io.c:writetable hands each cell, separator and
       newline to Rconn_printf with a literal "%s". Write those bytes
       straight to the connection: dispatching to the connection's vfprintf
       would re-scan the NUL-terminated string the formatter already
       produced (strlen) and copy it again before writing. Other formats,
       null strings and lengths beyond printf's int range keep the
       connection's formatter, exactly as m_vfprintf does for them. */
    if (format[0] == '%' && format[1] == 's' && format[2] == '\0') {
	va_list aq;
	const char *value;
	size_t n;
	va_copy(aq, ap);
	value = va_arg(aq, const char *);
	if (value != NULL && (n = strlen(value)) <= INT_MAX) {
	    va_end(aq);
	    R_WriteConnection(con, (void *) value, n);
	    va_end(ap);
	    return (int) n;
	}
	va_end(aq);
    }
    /* Parentheses added for FC4 with gcc4 and -D_FORTIFY_SOURCE=2 */
    res = (con->vfprintf)(con, format, ap);
    va_end(ap);
    /* PR#17243:  write.table and friends silently failed if the disk was full (or there was another error) */
    if (res < 0) {
	if (errno)
	    error(_("Error writing to connection:  %s"), strerror(errno));
	else
	    error(_("Error writing to connection"));
    }
    return res;
}

/* robigo: dounzip.c's unz() connection (R_newunz) builds a C connection
   with connections.c's init_con; robigo's connections live in the
   runtime, which serves unz() itself */
void Rf_init_con(Rconnection new, const char *description, int enc,
		 const char * const mode)
{
    error("robigo: C-level connections (init_con) are not implemented yet");
}

int dummy_fgetc(Rconnection con)
{
    error("robigo: C-level connections (dummy_fgetc) are not implemented yet");
    return -1;
}

/* R: unix/sys-std.c, the input handlers (the event loop sockets wait in)
   and the polled-events hook */
#include <R_ext/eventloop.h>
#include <R_ext/RS.h>
static InputHandler BasicInputHandler = {StdinActivity, -1, NULL};

/*
   This can be reset by the initialization routines which
   can ignore stdin, etc..
*/
InputHandler *R_InputHandlers = &BasicInputHandler;
InputHandler *
addInputHandler(InputHandler *handlers, int fd, InputHandlerProc handler,
		int activity)
{
    InputHandler *input, *tmp;
//    input = (InputHandler*) calloc(1, sizeof(InputHandler));
    input = R_Calloc(1, InputHandler);

    input->activity = activity;
    if (fd >= FD_SETSIZE)
	error("file descriptor is too large for select()");
    input->fileDescriptor = fd;
    input->handler = handler;

    tmp = handlers;

    if(handlers == NULL) {
	R_InputHandlers = input;
	return(input);
    }

    /* Go to the end of the list to append the new one.  */
    while(tmp->next != NULL) {
	tmp = tmp->next;
    }
    tmp->next = input;

    return(input);
}

/*
  Removes the specified handler from the linked list.

  See getInputHandler() for first locating the target handler instance.
 */
int
removeInputHandler(InputHandler **handlers, InputHandler *it)
{
    InputHandler *tmp;

    /* If the handler is the first one in the list, move the list to point
       to the second element. That's why we use the address of the first
       element as the first argument.
    */

    if (it == NULL) return(0);

    if(*handlers == it) {
	*handlers = (*handlers)->next;
	R_Free(it); // use R_Free to match allocation with R_Calloc
	return(1);
    }

    tmp = *handlers;

    while(tmp) {
	if(tmp->next == it) {
	    tmp->next = it->next;
	    R_Free(it); // use R_Free to match allocation with R_Calloc
	    return(1);
	}
	tmp = tmp->next;
    }

    return(0);
}


InputHandler *
getInputHandler(InputHandler *handlers, int fd)
{
    InputHandler *tmp;
    tmp = handlers;

    while(tmp != NULL) {
	if(tmp->fileDescriptor == fd)
	    return(tmp);
	tmp = tmp->next;
    }

    return(tmp);
}

/*
 Arrange to wait until there is some activity or input pending
 on one of the file descriptors to which we are listening.

 We could make the file descriptor mask persistent across
 calls and change it only when a listener is added or deleted.
 Later.

 This replaces the previous version which looked only on stdin and the
 X11 device connection.  This allows more than one X11 device to be
 open on a different connection. Also, it allows connections a la S4
 to be developed on top of this mechanism.
*/

/* A package can enable polled event handling by making R_PolledEvents
   point to a non-dummy routine and setting R_wait_usec to a suitable
   timeout value (e.g. 100000) */

static void nop(void){}

void (* R_PolledEvents)(void) = nop;
int R_wait_usec = 0; /* 0 means no timeout */
InputHandler *
getSelectedHandler(InputHandler *handlers, fd_set *readMask)
{
    InputHandler *tmp = handlers;

    /*
      Temporarily skip the first one if a) there is another one, and
      b) this is the BasicInputHandler.
    */
    if(handlers == &BasicInputHandler && handlers->next)
	tmp = handlers->next;

    while(tmp) {
	if(FD_ISSET(tmp->fileDescriptor, readMask))
	    return(tmp);
	tmp = tmp->next;
    }
    /* Now deal with the first one. */
    if(FD_ISSET(handlers->fileDescriptor, readMask))
	return(handlers);

    return((InputHandler*) NULL);
}
