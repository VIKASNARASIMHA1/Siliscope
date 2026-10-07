#include "bench.h"
#define UART ((volatile u8 *)0x10000000UL)
void putc_(char c) { while (!(UART[5] & 0x20)) {} UART[0] = c; }
void puts_(const char *s) { while (*s) putc_(*s++); }
void print_dec(u64 v) { char b[24]; int n = 0; do { b[n++] = '0' + v % 10; v /= 10; } while (v); while (n) putc_(b[--n]); }
void print_hex(u64 v) { for (int i = 60; i >= 0; i -= 4) putc_("0123456789abcdef"[(v >> i) & 15]); }
// libc entry points the compiler may emit calls to
void *memset(void *d, int c, unsigned long n) { u8 *p = d; while (n--) *p++ = c; return d; }
void *memcpy(void *d, const void *s, unsigned long n) { u8 *p = d; const u8 *q = s; while (n--) *p++ = *q++; return d; }
