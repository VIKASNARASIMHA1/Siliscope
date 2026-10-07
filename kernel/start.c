// Machine-mode setup: delegate everything to S-mode, program the timer, mret into kmain().
#include "kernel.h"

void kmain(void);
#define TICK_INTERVAL 50000L   // mtime ticks per scheduler tick (1 tick = 10 instructions)

static uint64 timer_scratch[5];

static void timerinit(void) {
  int id = 0;
  *(uint64 *)CLINT_MTIMECMP(id) = *(uint64 *)CLINT_MTIME + TICK_INTERVAL;
  timer_scratch[3] = CLINT_MTIMECMP(id);
  timer_scratch[4] = TICK_INTERVAL;
  w_mscratch((uint64)timer_scratch);
  w_mtvec((uint64)timervec);
  w_mstatus(r_mstatus() | MSTATUS_MIE);
  w_mie(r_mie() | MIE_MTIE);
}

void start(void) {
  uint64 x = r_mstatus();
  x &= ~MSTATUS_MPP_MASK;
  x |= MSTATUS_MPP_S;
  w_mstatus(x);
  w_mepc((uint64)kmain);
  w_satp(0);
  w_medeleg(0xffff);
  w_mideleg(0xffff);
  w_sie(r_sie() | SIE_SEIE | SIE_STIE | SIE_SSIE);
  w_pmpaddr0(0x3fffffffffffffL);
  w_pmpcfg0(0xf);
  timerinit();
  asm volatile("mret");
  for (;;) {}
}
