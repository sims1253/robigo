/* robigo's entry to TRE's approximate matchers (extra/README.md).

   tre_regaexecb and tre_regawexec take their regaparams_t by value, which
   the bridge's word trampoline (errors.rs:call_w6) cannot pass; this takes
   it by pointer and calls them with eflags 0, as src/main/agrep.c does. */

#include <stddef.h>
#include <wchar.h>
#include "tre.h"

int robigo_tre_aexec(const regex_t *preg, const void *string,
		     regamatch_t *match, const regaparams_t *params, int wide)
{
    if (wide)
	return tre_regawexec(preg, (const wchar_t *) string, match, *params, 0);
    return tre_regaexecb(preg, (const char *) string, match, *params, 0);
}
