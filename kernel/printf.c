#include "kernel.h"
#include <stdarg.h>

static char digits[] = "0123456789abcdef";

static void printint(long xx, int base, int sign) {
  char buf[24];
  int i = 0, neg = 0;
  unsigned long x;
  if (sign && xx < 0) { neg = 1; x = -xx; } else x = xx;
  do { buf[i++] = digits[x % base]; } while ((x /= base) != 0);
  if (neg) buf[i++] = '-';
  while (--i >= 0) uartputc_sync(buf[i]);
}

static void printptr(uint64 x) {
  uartputc_sync('0'); uartputc_sync('x');
  for (int i = 0; i < 16; i++, x <<= 4) uartputc_sync(digits[x >> 60]);
}

void printf(char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  for (int i = 0, c; (c = fmt[i] & 0xff) != 0; i++) {
    if (c != '%') { uartputc_sync(c); continue; }
    c = fmt[++i] & 0xff;
    if (c == 0) break;
    int lng = 0;
    if (c == 'l') { lng = 1; c = fmt[++i] & 0xff; }
    switch (c) {
    case 'd': printint(lng ? va_arg(ap, long) : va_arg(ap, int), 10, 1); break;
    case 'u': printint(lng ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int), 10, 0); break;
    case 'x': printint(lng ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int), 16, 0); break;
    case 'p': printptr(va_arg(ap, uint64)); break;
    case 's': { char *s = va_arg(ap, char *); if (!s) s = "(null)"; for (; *s; s++) uartputc_sync(*s); break; }
    case 'c': uartputc_sync(va_arg(ap, int)); break;
    case '%': uartputc_sync('%'); break;
    default: uartputc_sync('%'); uartputc_sync(c); break;
    }
  }
  va_end(ap);
}

void panic(char *s) {
  printf("\nkernel panic: %s\n", s);
  *(volatile uint32 *)FINISHER = (1 << 16) | 0x3333;  // power off with failure status
  for (;;) {}
}
