// 16550 UART driver (polled TX, interrupt-driven RX) and the console device.
#include "kernel.h"

#define Reg(r) ((volatile uchar *)(UART0 + (r)))
#define RHR 0
#define THR 0
#define IER 1
#define FCR 2
#define LCR 3
#define LSR 5

void uartinit(void) {
  *Reg(IER) = 0;
  *Reg(LCR) = 0x80; *Reg(0) = 3; *Reg(1) = 0;  // baud divisor
  *Reg(LCR) = 3;                                // 8N1
  *Reg(FCR) = 7;
  *Reg(IER) = 1;                                // RX interrupt
}

void uartputc_sync(int c) {
  while ((*Reg(LSR) & 0x20) == 0) {}
  *Reg(THR) = c;
}

int uartgetc(void) {
  if (*Reg(LSR) & 1) return *Reg(RHR);
  return -1;
}

void uartintr(void) {
  for (;;) {
    int c = uartgetc();
    if (c < 0) break;
    consoleintr(c);
  }
}

// ---- console: line-buffered input, no echo (the host terminal echoes) ------
#define INPUT_BUF 128
static struct { char buf[INPUT_BUF]; uint r, w, e; } cons;
#define CTRL(x) ((x) - '@')

void consoleintr(int c) {
  if (c == '\r') return;
  if (c == CTRL('P')) { procdump(); return; }
  if (cons.e - cons.r < INPUT_BUF) {
    cons.buf[cons.e++ % INPUT_BUF] = c;
    if (c == '\n' || c == CTRL('D') || cons.e - cons.r == INPUT_BUF) {
      cons.w = cons.e;
      wakeup(&cons.r);
    }
  }
}

static int consoleread(int user_dst, uint64 dst, int n) {
  int target = n;
  while (n > 0) {
    while (cons.r == cons.w) {
      if (curproc->killed) return -1;
      sleep(&cons.r);
    }
    int c = cons.buf[cons.r++ % INPUT_BUF];
    if (c == CTRL('D')) {
      if (n < target) cons.r--;  // return the bytes read so far; next read gives EOF
      break;
    }
    char cbuf = c;
    if (user_dst) { if (copyout(curproc->pagetable, dst, &cbuf, 1) < 0) break; }
    else *(char *)dst = cbuf;
    dst++; n--;
    if (c == '\n') break;
  }
  return target - n;
}

static int consolewrite(int user_src, uint64 src, int n) {
  int i;
  for (i = 0; i < n; i++) {
    char c;
    if (user_src) { if (copyin(curproc->pagetable, &c, src + i, 1) < 0) break; }
    else c = *(char *)(src + i);
    uartputc_sync(c);
  }
  return i;
}

void consoleinit(void) {
  uartinit();
  devsw[CONSOLE].read = consoleread;
  devsw[CONSOLE].write = consolewrite;
}
