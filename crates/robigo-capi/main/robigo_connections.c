/* robigo: GNU R 4.5.0's connections.c for connections C code makes
   (R_new_custom_connection, R_ext/Connections.h): init_con, the null_*
   and dummy_* methods, buff_fgetc and conFinalizer, copied from that
   file. The connection table lives in the runtime (crates/robigo-runtime,
   builtins/io.rs): where connections.c reads or writes Connections[] or
   current_id, this file calls robigo_* hooks (crates/robigo-capi/src/
   custom_conn.rs), marked "robigo:". */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#include <Defn.h>
#include <Rconnections.h>
#include <R_ext/Riconv.h>
#include <errno.h>

/* robigo: the runtime's connection table (custom_conn.rs) */
int robigo_next_connection(void);
void robigo_install_connection(int ncon, Rconnection con);
int robigo_connection_with_id(void *id);
void robigo_con_destroy(int ncon);
void *robigo_next_connection_id(void);
void robigo_forget_conn_id(SEXP ptr);

/* R: errors.c:Rvsnprintf_mbcs (the build without trio) */
attribute_hidden
int Rvsnprintf_mbcs(char *buf, size_t size, const char *format, va_list ap)
{
    int val;
    val = vsnprintf(buf, size, format, ap);
    if (size) {
	if (val < 0) buf[0] = '\0'; /* not all uses check val < 0 */
	else buf[size-1] = '\0';
	if (val >= size)
	    mbcsTruncateToValid(buf);
    }
    return val;
}

/* R: connections.c, the buffered fgetc (dummy_fgetc reads through it when con->buff is set) */
#define RBUFFCON_LEN_DEFAULT 4096
static void buff_reset(Rconnection con) {
    size_t unread_len = con->buff_stored_len - con->buff_pos;

    if (unread_len > 0)
	memmove(con->buff, con->buff + con->buff_pos, unread_len);

    con->buff_pos = 0;
    con->buff_stored_len = unread_len;
}

static size_t buff_fill(Rconnection con) {
    size_t free_len, read_len;

    buff_reset(con);

    free_len = con->buff_len - con->buff_stored_len;
    read_len = con->read(con->buff, sizeof(unsigned char), free_len, con);
    if ((int)read_len < 0)
	error("error reading from the connection");
    con->buff_stored_len += read_len;

    return read_len;
}

static int buff_fgetc(Rconnection con)
{
    size_t unread_len;

    unread_len = con->buff_stored_len - con->buff_pos;
    if (unread_len == 0) {
	size_t filled_len = buff_fill(con);
	if (filled_len == 0)
	    return R_EOF;
    }

    return con->buff[con->buff_pos++];
}

/* ------------------- null connection functions --------------------- */

NORET static Rboolean null_open(Rconnection con)
{
    error(_("%s not enabled for this connection"), "open");
}

static void null_close(Rconnection con)
{
    con->isopen = FALSE;
}

static void null_destroy(Rconnection con)
{
    if(con->private) free(con->private);
}

NORET static int null_vfprintf(Rconnection con, const char *format, va_list ap)
{
    error(_("%s not enabled for this connection"), "printing");
}

/* va_copy is C99, but a draft standard had __va_copy.  Glibc has
   __va_copy declared unconditionally */


#if defined(HAVE_VASPRINTF) && !HAVE_DECL_VASPRINTF
#endif

# define BUFSIZE 10000
// similar to Rcons_vprintf in printutils.c
int dummy_vfprintf(Rconnection con, const char *format, va_list ap)
{
    R_CheckStack2(BUFSIZE); // prudence
    char buf[BUFSIZE], *b = buf;
    int res;
    const void *vmax = NULL; /* -Wall*/
    int usedVasprintf = FALSE;
    va_list aq;

    va_copy(aq, ap);
    res = Rvsnprintf_mbcs(buf, BUFSIZE, format, aq);
    va_end(aq);
#ifdef HAVE_VASPRINTF
    if(res >= BUFSIZE || res < 0) {
	res = vasprintf(&b, format, ap);
	if (res < 0) {
	    b = buf;
	    warning(_("printing of extremely long output is truncated"));
	    res = (int)strlen(buf);
	} else usedVasprintf = TRUE;
    }
#else
    if(res >= BUFSIZE) { /* res is the desired output length */
	vmax = vmaxget();
	/* apparently some implementations count short,
	   <http://unixpapa.com/incnote/stdio.html>
	   so add some margin here */
	b = R_alloc(res + 101, sizeof(char));
	vsnprintf(b, res + 100, format, ap);
    } else if(res < 0) {
	/* Some non-C99 conforming vsnprintf implementations return -1 on
	   truncation instead of only on error. */
	vmax = vmaxget();
	b = R_alloc(10*BUFSIZE, sizeof(char));
	res = Rvsnprintf_mbcs(b, 10*BUFSIZE, format, ap);
	if (res < 0 || res >= 10*BUFSIZE) {
	    warning(_("printing of extremely long output is truncated"));
	    res = (int)strlen(b);
	}
    }
#endif /* HAVE_VASPRINTF */
    if(con->outconv) { /* translate the buffer */
	char outbuf[BUFSIZE+1], *ob;
	const char *ib = b;
	size_t inb = res, onb, ires;
	Rboolean again = FALSE;
	size_t ninit = strlen(con->init_out);
	do {
	    onb = BUFSIZE; /* leave space for nul */
	    ob = outbuf;
	    if(ninit) {
		strcpy(ob, con->init_out);
		ob += ninit; onb -= ninit; ninit = 0;
	    }
	    errno = 0;
	    ires = Riconv(con->outconv, &ib, &inb, &ob, &onb);
	    again = (ires == (size_t)(-1) && errno == E2BIG);
	    if(ires == (size_t)(-1) && errno != E2BIG) {
		Riconv(con->outconv, NULL, NULL, NULL, NULL);
		/* is this safe? */
		warning(_("invalid char string in output conversion"));
	    }
	    *ob = '\0';
	    con->write(outbuf, 1, ob - outbuf, con);
	} while(again && inb > 0);  /* it seems some iconv signal -1 on
				       zero-length input */
    } else
	con->write(b, 1, res, con);
    if(vmax) vmaxset(vmax);
    if(usedVasprintf) free(b);
    return res;
}

int dummy_fgetc(Rconnection con)
{
    if(con->inconv) {
	while(con->navail <= 0) {
	    unsigned int i, inew = 0;
	    char *p, *ob;
	    const char *ib;
	    size_t inb, onb, res;
	    Rboolean checkBOM = FALSE, checkBOM8 = FALSE;

	    if(con->EOF_signalled) return R_EOF;
	    if (con->inavail < 0) {
		switch(con->inavail) {
		case -2:
		    con->inavail = 0;
		    checkBOM = TRUE;
		    break;
		case -3:
		    con->inavail = 0;
		    checkBOM8 = TRUE;
		    break;
		case -21:
		    con->inavail = 1;
		    checkBOM = TRUE;
		    break;
		case -31:
		    con->inavail = 1;
		    checkBOM8 = TRUE;
		    break;
		case -32:
		    con->inavail = 2;
		    checkBOM8 = TRUE;
		    break;
		}
	    }
	    p = con->iconvbuff + con->inavail;
	    for(i = con->inavail; i < 25; i++) {
		int c = (con->buff)
		    ? buff_fgetc(con)
		    : con->fgetc_internal(con);
		if(c == R_EOF)
		    /* Do not set EOF_signalled, because subsequent reads from
		       a non-blocking connections may succeed (PR18555). */
		    break;
		*p++ = (char) c;
		con->inavail++;
		inew++;
		if(!con->buff && (c == '\n' || c == '\r'))
		    /* Possibly a line separator: better stop filling in the
		       encoding conversion buffer if not buffering the input
		       anyway, as not to confuse interactive applications
		       (PR17634).
		    */
		    break;
	    }
	    if (checkBOM || checkBOM8) {
		/* Handle the case of partial BOMs, e.g. in non-blocking
		   connections, when we do not have enough data to tell
		   whether there is a BOM or not.  Indicate by negative
		   con->inavail which BOM still needs to be checked and
		   how many bytes were already checked. */
		if(con->inavail == 0) {
		    if (checkBOM)
			con->inavail = -2;
		    else if (checkBOM8)
			con->inavail = -3;
		    return R_EOF;
		}
		if (con->inavail == 1) {
		    if (checkBOM && (((int)con->iconvbuff[0] & 0xff) == 255)) {
			con->inavail = -21;
			return R_EOF;
		    }
		    if (checkBOM8 && con->iconvbuff[0] == '\xef') {
			con->inavail = -31;
			return R_EOF;
		    }
		}
		if (con->inavail == 2 && checkBOM8 &&
		    con->iconvbuff[1] == '\xbb') {

		    con->inavail = -32;
		    return R_EOF;
		}
	    }
	    if(inew == 0) return R_EOF;
	    if(checkBOM && con->inavail >= 2 &&
	       ((int)con->iconvbuff[0] & 0xff) == 255 &&
	       ((int)con->iconvbuff[1] & 0xff) == 254) {
		con->inavail -= (short) 2;
		memmove(con->iconvbuff, con->iconvbuff+2, con->inavail);
	    }
	    if(checkBOM8 && con->inavail >= 3 &&
	       !memcmp(con->iconvbuff, "\xef\xbb\xbf", 3)) {
		con->inavail -= (short) 3;
		memmove(con->iconvbuff, con->iconvbuff+3, con->inavail);
	    }
	    ib = con->iconvbuff; inb = con->inavail;
	    ob = con->oconvbuff; onb = 50;
	    errno = 0;
	    res = Riconv(con->inconv, &ib, &inb, &ob, &onb);
	    con->inavail = (short) inb;
	    con->next = con->oconvbuff;
	    con->navail = (short)(50 - onb);
	    if(res == (size_t)-1) { /* an error condition */
		if(errno == EINVAL || errno == E2BIG) {
		    /* incomplete input char or no space in output buffer */
		    memmove(con->iconvbuff, ib, inb);
		} else {/*  EILSEQ invalid input */
		    Riconv(con->inconv, NULL, NULL, NULL, NULL);
		    warning(_("invalid input found on input connection '%s'"),
			    con->description);
		    con->inavail = 0;
		    /* Set to prevent reading any more bytes from input,
		       possibly those following the invalid bytes currently
		       encountered. */
		    con->EOF_signalled = TRUE;
		}
	    }
	}
	con->navail--;
	/* the cast prevents sign extension of 0xFF to -1 (R_EOF) */
	return (unsigned char)*con->next++;
    } else if (con->buff)
	return buff_fgetc(con);
    else
	return con->fgetc_internal(con);
}

NORET static int null_fgetc(Rconnection con)
{
    error(_("%s not enabled for this connection"), "'getc'");
}

NORET static double null_seek(Rconnection con, double where, int origin, int rw)
{
    error(_("%s not enabled for this connection"), "'seek'");
}

NORET static void null_truncate(Rconnection con)
{
    error(_("%s not enabled for this connection"), "truncation");
}

static int null_fflush(Rconnection con)
{
    return 0;
}

NORET static size_t null_read(void *ptr, size_t size, size_t nitems,
			      Rconnection con)
{
    error(_("%s not enabled for this connection"), "'read'");
}

NORET static size_t null_write(const void *ptr, size_t size, size_t nitems,
			       Rconnection con)
{
    error(_("%s not enabled for this connection"), "'write'");
}

/* R: connections.c:init_con */
void init_con(Rconnection new, const char *description, int enc,
	      const char * const mode)
{
    strcpy(new->description, description);
    new->enc = enc;
    strncpy(new->mode, mode, 4); new->mode[4] = '\0';
    new->isopen = new->incomplete = new->blocking = new->isGzcon = FALSE;
    new->canread = new->canwrite = TRUE; /* in principle */
    new->canseek = FALSE;
    new->text = TRUE;
    new->open = &null_open;
    new->close = &null_close;
    new->destroy = &null_destroy;
    new->vfprintf = &null_vfprintf;
    new->fgetc = new->fgetc_internal = &null_fgetc;
    new->seek = &null_seek;
    new->truncate = &null_truncate;
    new->fflush = &null_fflush;
    new->read = &null_read;
    new->write = &null_write;
    new->nPushBack = 0;
    new->save = new->save2 = -1000;
    new->private = NULL;
    new->inconv = new->outconv = NULL;
    new->UTF8out = FALSE;
    new->buff = NULL;
    new->buff_pos = new->buff_stored_len = new->buff_len = 0;
    /* increment id, avoid NULL (robigo: the runtime's counter, which every
       connection of the table takes its id from) */
    void *current_id = robigo_next_connection_id();
    new->id = current_id;
    new->ex_ptr = NULL;
    new->status = NA_INTEGER;
}

/* R: connections.c:conFinalizer */
static void conFinalizer(SEXP ptr)
{
    int ncon = 0;
    void *cptr = R_ExternalPtrAddr(ptr);

    /* robigo: no connection's con->ex_ptr may point to the dying pointer
       (GNU R keeps it dangling when C code replaced its address, as curl
       does, and nothing then finds the connection here) */
    robigo_forget_conn_id(ptr);
    if(!cptr) return;

    /* robigo: the search over Connections[], in the runtime's table */
    ncon = robigo_connection_with_id(cptr);
    if(ncon < 0) return;

    char buf[R_PATH_MAX + 50]; /* much longer than limit for warning */
    Rboolean warn = FALSE;
    {
	Rconnection this = getConnection(ncon);
	if(this->isopen && strcmp(this->class, "textConnection")) {
	    warn = TRUE;
	    snprintf(buf, sizeof(buf),
	             _("closing unused connection %d (%s)\n"),
	             ncon, this->description);
	}
    }

    robigo_con_destroy(ncon); /* robigo: con_destroy(ncon) */
    R_ClearExternalPtr(ptr); /* not really needed */

    if (warn)
	warning("%s", buf); /* may be turned into error */
}

SEXP R_new_custom_connection(const char *description, const char *mode, const char *class_name, Rconnection *ptr)
{
    Rconnection new;
    SEXP ans, class;

    int ncon = robigo_next_connection(); /* robigo: NextConnection() */

    /* built-in connections do this in a separate new<class>() function */
    new = (Rconnection) malloc(sizeof(struct Rconn));
    if(!new) error(_("allocation of %s connection failed"), class_name);
    new->class = (char *) malloc(strlen(class_name) + 1);
    if(!new->class) {
	free(new);
	error(_("allocation of %s connection failed"), class_name);
	/* for Solaris 12.5 */ new = NULL;
    }
    strcpy(new->class, class_name);
    new->description = (char *) malloc(strlen(description) + 1);
    if(!new->description) {
	free(new->class); free(new);
	error(_("allocation of %s connection failed"), class_name);
	/* for Solaris 12.5 */ new = NULL;
    }
    init_con(new, description, CE_NATIVE, mode);
    /* all ptrs are init'ed to null_* so no need to repeat that,
       but the following two are useful tools which could not be accessed otherwise */
    new->vfprintf = &dummy_vfprintf;
    new->fgetc = &dummy_fgetc;

    /* here we use the new connection to create a SEXP */
    robigo_install_connection(ncon, new); /* robigo: Connections[ncon] = new; */
    /* new->blocking = block; */
    new->encname[0] = 0; /* "" (should have the same effect as "native.enc") */
    new->ex_ptr = PROTECT(R_MakeExternalPtr(new->id, install("connection"), R_NilValue));

    PROTECT(ans = ScalarInteger(ncon));
    PROTECT(class = allocVector(STRSXP, 2));
    SET_STRING_ELT(class, 0, mkChar(class_name));
    SET_STRING_ELT(class, 1, mkChar("connection"));
    classgets(ans, class);
    setAttrib(ans, install("conn_id"), new->ex_ptr); /* robigo: R_ConnIdSymbol */
    R_RegisterCFinalizerEx(new->ex_ptr, conFinalizer, FALSE);
    UNPROTECT(3);

    if (ptr) ptr[0] = new;

    return ans;
}

/* robigo: con->vfprintf(con, "%s", s), for text the runtime formatted
   (Rconn_printf's call of the method; con->vfprintf takes a va_list) */
static int robigo_call_vfprintf(Rconnection con, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    int res = con->vfprintf(con, format, ap);
    va_end(ap);
    return res;
}

int robigo_conn_vfprintf(Rconnection con, const char *s)
{
    return robigo_call_vfprintf(con, "%s", s);
}

/* robigo: what con_destroy does after checkClose for a connection C code
   made: con_close1's con->destroy and frees, then con_destroy's free
   (R: connections.c:con_close1, con_destroy). Pushback and the read
   buffer belong to the runtime's connection. */
void robigo_destroy_connection(Rconnection con)
{
    /* close inconv and outconv if open */
    if(con->inconv) Riconv_close(con->inconv);
    if(con->outconv) Riconv_close(con->outconv);
    con->destroy(con);
    free(con->class);
    con->class = NULL;
    free(con->description);
    con->description = NULL;
    if (con->buff) {
	free(con->buff);
	con->buff = NULL;
    }
    free(con);
}
