// Trap handling: system calls, device interrupts, timer preemption.
#include "kernel.h"

extern struct proc proc[];

uint64 ticks;
static const int slice_ticks[NLEVELS] = {1, 2, 4};

void trapinit(void) { w_stvec((uint64)trapvec); }

static void clockintr(void) {
  ticks++;
  if (sched_mlfq && ticks % BOOST_TICKS == 0)        // periodic priority boost prevents starvation
    for (struct proc *p = proc; p < &proc[NPROC]; p++) p->prio = 0;
  wakeup(&ticks);
}

// returns 0 = not a device interrupt, 1 = external device, 2 = timer tick
static int devintr(void) {
  uint64 scause = r_scause();
  if ((scause & (1UL << 63)) && (scause & 0xff) == 9) {
    int irq = plic_claim();
    if (irq == UART0_IRQ) uartintr();
    else if (irq == VIRTIO0_IRQ) virtio_disk_intr();
    else if (irq) printf("unexpected interrupt irq=%d\n", irq);
    if (irq) plic_complete(irq);
    return 1;
  }
  if (scause == ((1UL << 63) | 1)) {
    clockintr();
    w_sip(r_sip() & ~2UL);
    return 2;
  }
  return 0;
}

static int handle_pagefault(struct proc *p, uint64 scause, uint64 va) {
  if (scause == 15 && cowfault(p->pagetable, va) == 0) return 1;
  return vmfault(p->pagetable, va) != 0;
}

void usertrap(struct trapframe *tf) {
  struct proc *p = curproc;
  if (tf != &p->tf) panic("usertrap: bad trapframe");
  w_stvec((uint64)trapvec);
  tf->sepc = r_sepc();
  uint64 scause = r_scause();
  int which = 0;
  if (scause == 8) {
    if (p->killed) exit(-1);
    tf->sepc += 4;
    syscall();
  } else if ((which = devintr()) != 0) {
    // handled
  } else if ((scause == 13 || scause == 15) && handle_pagefault(p, scause, r_stval())) {
    // copy-on-write / lazy allocation / swap-in resolved; the faulting access is retried
  } else {
    printf("usertrap: unexpected scause %lx pid=%d (%s) sepc=%lx stval=%lx -- killing process\n",
           scause, p->pid, p->name, r_sepc(), r_stval());
    p->killed = 1;
  }
  if (p->killed) exit(-1);
  if (which == 2) {
    p->cpu_ticks++;
    if (++p->used >= slice_ticks[p->prio]) {
      p->used = 0;
      if (sched_mlfq && p->prio < NLEVELS - 1) p->prio++;   // used its whole slice: demote
      yield();
    } else if (sched_mlfq && higher_prio_runnable(p->prio)) {
      yield();                                                // a higher-priority job just woke: preempt
    }
  }
  usertrapret();
}

void usertrapret(void) {
  struct proc *p = curproc;
  intr_off();
  w_stvec((uint64)trapvec);
  p->tf.ksp = (uint64)p->kstack + KSTACKSZ;
  uint64 x = r_sstatus();
  x &= ~SSTATUS_SPP;        // return to user mode
  x |= SSTATUS_SPIE;        // enable interrupts there
  w_sstatus(x);
  w_sepc(p->tf.sepc);
  w_sscratch((uint64)&p->tf);
  userret(&p->tf);
  for (;;) {}
}

// Trap taken while in the kernel: only the idle loop enables interrupts.
void kerneltrap(void) {
  uint64 sepc = r_sepc(), sstatus = r_sstatus(), scause = r_scause();
  if (!(sstatus & SSTATUS_SPP)) panic("kerneltrap: not from supervisor mode");
  if (devintr() == 0) {
    printf("scause %lx sepc=%lx stval=%lx\n", scause, sepc, r_stval());
    panic("kerneltrap");
  }
  w_sepc(sepc);
  w_sstatus(sstatus);
}
