#include "kernel.h"

void *memset(void *dst, int c, uint n) {
  char *d = dst;
  if (((uint64)d & 7) == 0 && n >= 8) {            // word-at-a-time fast path
    uint64 w = (uchar)c; w |= w << 8; w |= w << 16; w |= w << 32;
    uint64 *p = (uint64 *)d;
    uint words = n / 8;
    for (uint i = 0; i < words; i++) p[i] = w;
    d += words * 8; n -= words * 8;
  }
  for (uint i = 0; i < n; i++) d[i] = c;
  return dst;
}
int memcmp(const void *a, const void *b, uint n) {
  const uchar *s1 = a, *s2 = b;
  while (n-- > 0) { if (*s1 != *s2) return *s1 - *s2; s1++; s2++; }
  return 0;
}
void *memmove(void *dst, const void *src, uint n) {
  const char *s = src; char *d = dst;
  if (s < d && s + n > d) { s += n; d += n; while (n-- > 0) *--d = *--s; }
  else {
    if ((((uint64)s | (uint64)d) & 7) == 0) {       // word-at-a-time fast path
      const uint64 *ws = (const uint64 *)s; uint64 *wd = (uint64 *)d;
      while (n >= 8) { *wd++ = *ws++; n -= 8; }
      s = (const char *)ws; d = (char *)wd;
    }
    while (n-- > 0) *d++ = *s++;
  }
  return dst;
}
void *memcpy(void *dst, const void *src, uint n) { return memmove(dst, src, n); }
int strlen(const char *s) { int n = 0; while (s[n]) n++; return n; }
int strcmp(const char *p, const char *q) { while (*p && *p == *q) p++, q++; return (uchar)*p - (uchar)*q; }
int strncmp(const char *p, const char *q, uint n) {
  while (n > 0 && *p && *p == *q) n--, p++, q++;
  return n == 0 ? 0 : (uchar)*p - (uchar)*q;
}
char *strncpy(char *s, const char *t, int n) {
  char *os = s;
  while (n-- > 0 && (*s++ = *t++) != 0) {}
  while (n-- > 0) *s++ = 0;
  return os;
}
char *safestrcpy(char *s, const char *t, int n) {
  char *os = s;
  if (n <= 0) return os;
  while (--n > 0 && (*s++ = *t++) != 0) {}
  *s = 0;
  return os;
}
