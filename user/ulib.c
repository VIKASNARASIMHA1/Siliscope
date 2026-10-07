#include "user.h"
#include <stdarg.h>

int main(int, char **);
void _start(int argc, char **argv) { exit(main(argc, argv)); }

int strlen(const char *s) { int n = 0; while (s[n]) n++; return n; }
int strcmp(const char *p, const char *q) { while (*p && *p == *q) p++, q++; return (uchar)*p - (uchar)*q; }
char *strcpy(char *s, const char *t) { char *os = s; while ((*s++ = *t++) != 0) {} return os; }
char *strchr(const char *s, char c) { for (; *s; s++) if (*s == c) return (char *)s; return 0; }
void *memset(void *dst, int c, uint n) { char *d = dst; for (uint i = 0; i < n; i++) d[i] = c; return dst; }
void *memmove(void *dst, const void *src, uint n) {
  const char *s = src; char *d = dst;
  if (s < d && s + n > d) { s += n; d += n; while (n-- > 0) *--d = *--s; }
  else while (n-- > 0) *d++ = *s++;
  return dst;
}
void *memcpy(void *dst, const void *src, uint n) { return memmove(dst, src, n); }
int memcmp(const void *a, const void *b, uint n) {
  const uchar *p = a, *q = b;
  for (; n > 0; n--, p++, q++) if (*p != *q) return *p - *q;
  return 0;
}
int atoi(const char *s) { int n = 0, neg = 0; if (*s == '-') { neg = 1; s++; } while (*s >= '0' && *s <= '9') n = n * 10 + *s++ - '0'; return neg ? -n : n; }

char *gets(char *buf, int max) {
  int i = 0;
  while (i + 1 < max) {
    char c;
    if (read(0, &c, 1) < 1) break;
    buf[i++] = c;
    if (c == '\n') break;
  }
  buf[i] = 0;
  return buf;
}

// ---- printf: formats into a buffer, then issues one write() ----------------------
static int putnum(char *out, unsigned long x, int base, int sign, int neg) {
  static const char digits[] = "0123456789abcdef";
  char tmp[24];
  int n = 0, o = 0;
  (void)sign;
  do { tmp[n++] = digits[x % base]; } while ((x /= base) != 0);
  if (neg) tmp[n++] = '-';
  while (n > 0) out[o++] = tmp[--n];
  return o;
}

static void vfprintf(int fd, const char *fmt, va_list ap) {
  char buf[256];
  int o = 0;
  for (int i = 0; fmt[i]; i++) {
    if (o > 200) { write(fd, buf, o); o = 0; }
    char c = fmt[i];
    if (c != '%') { buf[o++] = c; continue; }
    c = fmt[++i];
    int lng = 0;
    if (c == 'l') { lng = 1; c = fmt[++i]; }
    switch (c) {
    case 'd': { long v = lng ? va_arg(ap, long) : va_arg(ap, int); o += putnum(buf + o, v < 0 ? -v : v, 10, 1, v < 0); break; }
    case 'u': o += putnum(buf + o, lng ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int), 10, 0, 0); break;
    case 'x': o += putnum(buf + o, lng ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int), 16, 0, 0); break;
    case 's': { const char *s = va_arg(ap, const char *); if (!s) s = "(null)"; while (*s) { if (o > 250) { write(fd, buf, o); o = 0; } buf[o++] = *s++; } break; }
    case 'c': buf[o++] = va_arg(ap, int); break;
    case '%': buf[o++] = '%'; break;
    default: buf[o++] = '%'; buf[o++] = c; break;
    }
  }
  if (o) write(fd, buf, o);
}

void fprintf(int fd, const char *fmt, ...) { va_list ap; va_start(ap, fmt); vfprintf(fd, fmt, ap); va_end(ap); }
void printf(const char *fmt, ...) { va_list ap; va_start(ap, fmt); vfprintf(1, fmt, ap); va_end(ap); }
