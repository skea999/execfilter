/*
 * execl_shim.c — the only part of execfilter that cannot be stable Rust:
 * C-variadic function definitions (execl/execlp) are nightly-only in Rust
 * (std::ffi::VaList), so these two hooks stay in C.
 *
 * glibc implements execl/execlp on top of the internal __execve, which
 * bypasses the Rust execve hook — they must be intercepted explicitly and
 * routed through our own execv/execvp (same as the original execfilter.c).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <stdarg.h>
#include <stdlib.h>

extern int execv(const char *path, char *const argv[]);
extern int execvp(const char *file, char *const argv[]);

/* Rebuild argv from the varargs; NULL-terminated. */
static char **collect(const char *arg0, va_list ap)
{
    size_t n = 1;
    va_list cp;
    va_copy(cp, ap);
    while (va_arg(cp, char *))
        n++;
    va_end(cp);

    char **argv = calloc(n + 1, sizeof(char *));
    if (!argv) {
        errno = ENOMEM;
        return NULL;
    }
    argv[0] = (char *)arg0;
    for (size_t i = 1; i < n; i++)
        argv[i] = va_arg(ap, char *);
    argv[n] = NULL;
    return argv;
}

int execl(const char *path, const char *arg, ...)
{
    va_list ap;
    va_start(ap, arg);
    char **argv = collect(arg, ap);
    va_end(ap);
    if (!argv)
        return -1;
    return execv(path, argv);
}

int execlp(const char *file, const char *arg, ...)
{
    va_list ap;
    va_start(ap, arg);
    char **argv = collect(arg, ap);
    va_end(ap);
    if (!argv)
        return -1;
    return execvp(file, argv);
}
