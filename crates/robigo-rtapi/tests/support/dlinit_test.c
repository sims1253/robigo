/* A shared object whose constructor and destructor stop inside `dlopen`
 * and `dlclose` until robigo-rtapi's dynload test lets them go on
 * (tests/dynload.rs, a_librarys_initializers_and_finalizers_hold_env).
 * Only when the file `<object>.armed` exists: each creates
 * `<object>.ready`, waits for `<object>.go` (60 s at most, so that a failed
 * test never hangs), then removes both. The test checks the environment's
 * lock in between. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

/* `<path><suffix>` into buf, or 0 (said on stderr) when it does not fit */
static int marker(char *buf, size_t size, const char *path, const char *suffix)
{
    int n = snprintf(buf, size, "%s%s", path, suffix);
    if (n < 0 || (size_t)n >= size) {
        fprintf(stderr, "dlinit_test: the path of %s%s does not fit\n", path, suffix);
        return 0;
    }
    return 1;
}

static void handshake(void)
{
    Dl_info i;
    char armed[4096], ready[4096], go[4096];
    struct timespec ms = {0, 1000000};
    int fd;
    if (!dladdr((void *)handshake, &i) || !i.dli_fname) {
        fprintf(stderr, "dlinit_test: dladdr found no path for the object\n");
        return;
    }
    if (!marker(armed, sizeof armed, i.dli_fname, ".armed") || !marker(ready, sizeof ready, i.dli_fname, ".ready")
        || !marker(go, sizeof go, i.dli_fname, ".go"))
        return;
    if (access(armed, F_OK) != 0)
        return;
    fd = open(ready, O_WRONLY | O_CREAT, 0600);
    if (fd < 0) {
        perror("dlinit_test: creating the .ready marker");
        return;
    }
    close(fd);
    for (int k = 0; k < 60000 && access(go, F_OK) != 0; k++)
        nanosleep(&ms, NULL);
    unlink(ready);
    unlink(go);
}

__attribute__((constructor)) static void dlinit_ctor(void) { handshake(); }
__attribute__((destructor)) static void dlinit_dtor(void) { handshake(); }
