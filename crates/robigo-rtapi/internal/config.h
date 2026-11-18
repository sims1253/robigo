/* robigo's config.h for GNU R's base-package natives (docs/g3.md §10): the
 * values GNU R 4.6.2's configure gives on Linux x86-64 (glibc) for the
 * macros the natives robigo builds test, and no ENABLE_NLS (robigo has no
 * message catalogs: `_()` is the identity, GNU R's text in the C locale).
 * A package's natives add the macros they test. */
#ifndef ROBIGO_CONFIG_H_
#define ROBIGO_CONFIG_H_

/* tools */
#define HAVE_DIRENT_H 1
#define HAVE_GETPRIORITY 1
#define HAVE_KILL 1
#define HAVE_LIMITS_H 1
#define HAVE_SYS_RESOURCE_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_SYS_TYPES_H 1

#endif /* ROBIGO_CONFIG_H_ */
