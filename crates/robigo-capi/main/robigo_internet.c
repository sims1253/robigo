/* robigo: GNU R 4.5.0's src/main/internet.c socket entry points
   (Rsockconnect ... Rsockwrite, copied from there). GNU R reaches the
   internet module's socket routines through R_InternetRoutines after
   loading the module; robigo links the module's Rsock.c and sock.c
   (vendored here) into libR, so the table holds them from the start. */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#include <Defn.h>

void in_Rsockopen(int *port);
void in_Rsocklisten(int *sockp, char **buf, int *len);
void in_Rsockconnect(int *port, char **host);
void in_Rsockclose(int *sockp);
void in_Rsockread(int *sockp, char **buf, int *maxlen);
void in_Rsockwrite(int *sockp, char **buf, int *start, int *end, int *len);

static struct {
    void (*sockopen)(int *);
    void (*socklisten)(int *, char **, int *);
    void (*sockconnect)(int *, char **);
    void (*sockclose)(int *);
    void (*sockread)(int *, char **, int *);
    void (*sockwrite)(int *, char **, int *, int *, int *);
    SEXP (*download)(SEXP);
} routines, *ptr = &routines;

#include <Rconnections.h>
#include <Fileio.h>
#include <errno.h>

/* R: modules/internet/internet.c (the Unix parts; Win32 code is compiled
   out as there) */
#define CPBUFSIZE 65536
static Rboolean IDquiet = TRUE;

static SEXP in_do_download(SEXP args)
{
    SEXP scmd, sfile, smode, sheaders;
    const char *url, *file, *mode;
    int quiet, status = 0, cacheOK, meth = 0;
#ifdef Win32
    char pbuf[30];
    int pc;
#endif

    scmd = CAR(args); args = CDR(args);
    if(!isString(scmd) || length(scmd) < 1)
	error(_("invalid '%s' argument"), "url");
    if(length(scmd) > 1)
	warning(_("only first element of 'url' argument used"));
    url = CHAR(STRING_ELT(scmd, 0));
    sfile = CAR(args); args = CDR(args);
    if(!isString(sfile) || length(sfile) < 1)
	error(_("invalid '%s' argument"), "destfile");
    if(length(sfile) > 1)
	warning(_("only first element of 'destfile' argument used"));
    file = translateChar(STRING_ELT(sfile, 0));
    IDquiet = quiet = asRbool(CAR(args), R_NilValue); args = CDR(args);
    if(quiet == NA_LOGICAL)
	error(_("invalid '%s' argument"), "quiet");
    smode =  CAR(args); args = CDR(args);
    if(!isString(smode) || length(smode) != 1)
	error(_("invalid '%s' argument"), "mode");
    mode = CHAR(STRING_ELT(smode, 0));
    cacheOK = asLogical(CAR(args)); args = CDR(args);
    if(cacheOK == NA_LOGICAL)
	error(_("invalid '%s' argument"), "cacheOK");
    Rboolean file_URL = (strncmp(url, "file://", 7) == 0);
    sheaders = CAR(args);
    if(TYPEOF(sheaders) != NILSXP && !isString(sheaders))
        error(_("invalid '%s' argument"), "headers");
#ifdef Win32
    meth = asLogical(CADR(args));
    if(meth == NA_LOGICAL)
	error(_("invalid '%s' argument"), "method");
    if (!file_URL && R_Interactive && !quiet && !pbar.wprog) {
	pbar.wprog = newwindow(_("Download progress"), rect(0, 0, 540, 100),
			       Titlebar | Centered);
	setbackground(pbar.wprog, dialog_bg());
	pbar.l_url = newlabel(" ", rect(10, 15, 520, 25), AlignCenter);
	pbar.pb = newprogressbar(rect(20, 50, 500, 20), 0, 1024, 1024, 1);
	pbar.pc = 0;
    }
#endif
    if(file_URL) {
// ---------  file:// code ---------------
	FILE *in, *out;
	static char buf[CPBUFSIZE];
	size_t n;
	int nh = 7, mlen;
#ifdef Win32
	/* on Windows we have file:///d:/path/to
	   whereas on Unix it is file:///path/to */
	if (strlen(url) > 9 && url[7] == '/' && url[9] == ':') nh = 8;
#endif

	/* Use binary transfers? */
	mlen = (int) strlen(mode);
	in = R_fopen(R_ExpandFileName(url+nh),
	             (mlen >= 2 && mode[mlen - 1] == 'b') ? "rb" : "r");
	if(!in) {
	    error(_("cannot open URL '%s', reason '%s'"),
		  url, strerror(errno));
	}

	out = R_fopen(R_ExpandFileName(file), mode);
	if(!out) {
	    fclose(in);
	    error(_("cannot open destfile '%s', reason '%s'"),
		  file, strerror(errno));
	}
	while((n = fread(buf, 1, CPBUFSIZE, in)) > 0) {
	    size_t res = fwrite(buf, 1, n, out);
	    if(res != n) error(_("write failed"));
	}
	fclose(out); fclose(in);
// ---------  end of file:// code ---------------

    } else if(!meth && strncmp(url, "http://", 7) == 0) {
	error(_("the 'internal' method for http:// URLs is defunct"));
#ifdef Win32
// ---------  wininet only code ---------------
    } else if (meth &&
	       (strncmp(url, "http://", 7) == 0
		|| (strncmp(url, "https://", 8) == 0))
	) {

	warning(_("the 'wininet' method is deprecated for http:// and https:// URLs"));
	FILE *out;
	void *ctxt;
	DLsize_t len, total, guess, nbytes = 0;
	char buf[IBUFSIZE];
	int ndashes = 0;
	DLsize_t ndots = 0;
	int factor = 1;

	out = R_fopen(R_ExpandFileName(file), mode);
	if(!out) {
	    error(_("cannot open destfile '%s', reason '%s'"),
		  file, strerror(errno));
	}

	R_Busy(1);
	if(!quiet) REprintf(_("trying URL '%s'\n"), url);
	SEXP agentFun, sagent;
	R_FlushConsole();
	agentFun = PROTECT(lang2(install("makeUserAgent"), ScalarLogical(0)));
	SEXP utilsNS = PROTECT(R_FindNamespace(mkString("utils")));
	sagent = eval(agentFun, utilsNS);
	UNPROTECT(1); /* utilsNS */
	PROTECT(sagent);
	const char *cagent = (TYPEOF(sagent) == NILSXP) ?
	    NULL : CHAR(STRING_ELT(sagent, 0));
	/* TODO: flatten headers */
	const char *cheaders = (TYPEOF(sheaders) == NILSXP) ?
	    NULL : CHAR(STRING_ELT(sheaders, 0));
	ctxt = in_R_HTTPOpen2(url, cagent, cheaders, cacheOK);
	UNPROTECT(2);
	if(ctxt == NULL) status = 1;
	else {
//	    if(!quiet) REprintf(_("opened URL\n"), url);
	    guess = total = ((inetconn *)ctxt)->length;
	    if(R_Interactive) {
		if (guess <= 0) guess = 100 * 1024;
		if (guess > 1e9) factor = guess/1e6;
		R_FlushConsole();
		strcpy(buf, "URL: ");
		if(strlen(url) > 60) {
		    strcat(buf, "... ");
		    strcat(buf, url + (strlen(url) - 60));
		} else strcat(buf, url);
		if(!quiet) {
		    settext(pbar.l_url, buf);
		    setprogressbarrange(pbar.pb, 0, guess/factor);
		    setprogressbar(pbar.pb, 0);
		    settext(pbar.wprog, "Download progress");
		    show(pbar.wprog);
		    begincontext(&(pbar.cntxt), CTXT_CCODE, R_NilValue, R_NilValue,
				 R_NilValue, R_NilValue, R_NilValue);
		    pbar.cntxt.cend = &doneprogressbar;
		    pbar.cntxt.cenddata = &pbar;
		    pbar.pc = 0;
		}
	    }
	    while ((len = in_R_HTTPRead2(ctxt, buf, sizeof(buf))) > 0) {
		size_t res = fwrite(buf, 1, len, out);
		if(res != len) error(_("write failed"));
		nbytes += len;
		if(!quiet) {
		    if(R_Interactive) {
			if(nbytes > guess) {
			    guess *= 2;
			    if (guess > 1e9) factor = guess/1e6;
			    setprogressbarrange(pbar.pb, 0, guess/factor);
			}
			setprogressbar(pbar.pb, nbytes/factor);
			if (total > 0) {
			    pc = 0.499 + 100.0*nbytes/total;
			    if (pc > pbar.pc) {
				snprintf(pbuf, 30, "%d%% downloaded", pc);
				settext(pbar.wprog, pbuf);
				pbar.pc = pc;
			    }
			}
		    } else {
			if(guess <= 0) putdots(&ndots, nbytes/1024);
			else putdashes(&ndashes, (int)(50*nbytes/guess));
		    }
		}
	    }
	    in_R_HTTPClose2(ctxt);
	    if(!quiet) {
		if(!R_Interactive) REprintf("\n");
		if(nbytes > 1024*1024)
		    REprintf("downloaded %0.1f MB\n\n",
			     (double)nbytes/1024/1024);
		else if(nbytes > 10240)
		    REprintf("downloaded %d KB\n\n", (int) nbytes/1024);
		else
		    REprintf("downloaded %d bytes\n\n", (int) nbytes);
	    }
	    R_FlushConsole();
	    if(R_Interactive && !quiet) {
		endcontext(&(pbar.cntxt));
		doneprogressbar(&pbar);
	    }
	    if (total > 0 && total != nbytes)
		warning(_("downloaded length %0.f != reported length %0.f"),
			(double)nbytes, (double)total);
	}
	fclose(out);
	if (status == 1 && strchr(mode, 'w')) unlink(R_ExpandFileName(file));
	R_Busy(0);
	if (status == 1) error(_("cannot open URL '%s'"), url);
// ---------  end of wininet only code ---------------
#endif
    } else if (strncmp(url, "ftp://", 6) == 0) {
	if(meth)
	    error(_("the 'wininet' method for ftp:// URLs is defunct"));
	else
	    error(_("the 'internal' method for ftp:// URLs is defunct"));
   } else
	error(_("scheme not supported in URL '%s'"), url);

    return ScalarInteger(status);
}

static int initialized = 1;
static void internet_Init(void) {}

static void __attribute__((constructor)) robigo_internet_routines(void)
{
    routines.sockopen = in_Rsockopen;
    routines.socklisten = in_Rsocklisten;
    routines.sockconnect = in_Rsockconnect;
    routines.sockclose = in_Rsockclose;
    routines.sockread = in_Rsockread;
    routines.sockwrite = in_Rsockwrite;
    routines.download = in_do_download;
}

/* R: main/internet.c:Rdownload */
SEXP Rdownload(SEXP args)
{
    if(!initialized) internet_Init();
    if(initialized > 0)
	return (*ptr->download)(args);
    else {
	error(_("internet routines cannot be loaded"));
	return R_NilValue;
    }
}

SEXP Rsockconnect(SEXP sport, SEXP shost)
{
    if (length(sport) != 1) error("invalid 'socket' argument");
    int port = asInteger(sport);
    char *host[1];
    host[0] = (char *) translateCharFP(STRING_ELT(shost, 0));
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->sockconnect)(&port, host);
    else
	error(_("socket routines cannot be loaded"));
    return ScalarInteger(port); // The socket number
}

SEXP Rsockread(SEXP ssock, SEXP smaxlen)
{
    if (length(ssock) != 1) error("invalid 'socket' argument");
    int sock = asInteger(ssock);
    int maxlen = asInteger(smaxlen);
    if (maxlen < 0) /* also catches NA_INTEGER */
	error(_("maxlen must be non-negative"));
    SEXP rbuf = allocVector(RAWSXP, maxlen + 1);
    PROTECT(rbuf);
    char *buf = (char *) RAW(rbuf), *abuf[1];
    abuf[0] = buf;
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->sockread)(&sock, abuf, &maxlen);
    else
	error(_("socket routines cannot be loaded"));
    if (maxlen < 0) // presumably -1, error from recv
	error("Error reading data in Rsockread");
    SEXP ans = PROTECT(allocVector(STRSXP, 1));
    SET_STRING_ELT(ans, 0, mkCharLen(buf, maxlen));
    UNPROTECT(2); /* rbuf, ans */
    return ans;
		       
}

SEXP Rsockclose(SEXP ssock)
{
    if (length(ssock) != 1) error("invalid 'socket' argument");
    int sock = asInteger(ssock);
    if (sock <= 0) error(_("attempt to close invalid socket"));
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->sockclose)(&sock);
    else
	error(_("socket routines cannot be loaded"));
    return ScalarLogical(sock);
}

SEXP Rsockopen(SEXP sport)
{
    if (length(sport) != 1) error("invalid 'port' argument");
    int port = asInteger(sport);
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->sockopen)(&port);
    else
	error(_("socket routines cannot be loaded"));
    return ScalarInteger(port); // The socket number
}

SEXP Rsocklisten(SEXP ssock)
{
    if (length(ssock) != 1) error("invalid 'socket' argument");
    int sock = asInteger(ssock), len = 256;
    char buf[257], *abuf[1];
    abuf[0] = buf;
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->socklisten)(&sock, abuf, &len);
    else
	error(_("socket routines cannot be loaded"));
    SEXP ans = PROTECT(ScalarInteger(sock)); // The socket being listened on
    SEXP host = PROTECT(allocVector(STRSXP, 1));
    SET_STRING_ELT(host, 0, mkChar(buf));
    setAttrib(ans, install("host"), host);
    UNPROTECT(2);
    return ans;
}

SEXP Rsockwrite(SEXP ssock, SEXP sstring)
{
    if (length(ssock) != 1) error("invalid 'socket' argument");
    int sock = asInteger(ssock), start = 0, end, len;
    char *buf = (char *) translateCharFP(STRING_ELT(sstring, 0)), *abuf[1];
    end = len = (int) strlen(buf);
    abuf[0] = buf;
    if(!initialized) internet_Init();
    if(initialized > 0)
	(*ptr->sockwrite)(&sock, abuf, &start, &end, &len);
    else
	error(_("socket routines cannot be loaded"));
    return ScalarInteger(len);
}
