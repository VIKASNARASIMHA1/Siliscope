// Processes, the scheduler (round-robin or MLFQ), sleep/wakeup, fork/exit/wait.
#include "kernel.h"

struct proc proc[NPROC];
struct proc *curproc;
static struct proc *initproc;
static struct context sched_ctx;
static int nextpid = 1;
static int rr_last[NLEVELS];
int sched_mlfq = SCHED_MLFQ_DEFAULT;
__attribute__((aligned(16))) static char kstacks[NPROC][KSTACKSZ];


void procinit(void) {
  for (int i = 0; i < NPROC; i++) { proc[i].state = UNUSED; proc[i].kstack = kstacks[i]; }
}

static void forkret(void) {
  static int first = 1;
  if (first) {
    first = 0;
    fsinit(1);
    curproc->cwd = namei("/");
    char *argv[] = {"/init", 0};
    if (exec("/init", argv) < 0) panic("forkret: cannot exec /init (is fs.img attached?)");
  }
  usertrapret();
}

static struct proc *allocproc(void) {
  struct proc *p;
  for (p = proc; p < &proc[NPROC]; p++) if (p->state == UNUSED) goto found;
  return 0;
found:
  p->pid = nextpid++;
  p->state = USED;
  p->killed = 0; p->xstate = 0; p->chan = 0; p->prio = 0; p->used = 0;
  p->cpu_ticks = 0; p->nsched = 0;
  memset(&p->tf, 0, sizeof p->tf);
  if ((p->pagetable = uvmcreate()) == 0) { p->state = UNUSED; return 0; }
  p->sz = UBASE;
  memset(&p->ctx, 0, sizeof p->ctx);
  p->ctx.ra = (uint64)forkret;
  p->ctx.sp = (uint64)p->kstack + KSTACKSZ;
  p->tf.ksp = (uint64)p->kstack + KSTACKSZ;
  return p;
}

static void freeproc(struct proc *p) {
  if (p->pagetable) uvmfree(p->pagetable, p->sz);
  p->pagetable = 0; p->sz = 0; p->pid = 0; p->parent = 0; p->name[0] = 0; p->chan = 0; p->killed = 0;
  p->state = UNUSED;
}

void userinit(void) {
  struct proc *p = allocproc();
  initproc = p;
  safestrcpy(p->name, "init", sizeof p->name);
  p->state = RUNNABLE;
}

int growproc(int n) {
  struct proc *p = curproc;
  uint64 sz = p->sz;
  if (n > 0) {
    if (sz + n >= USTACKBASE - PGSIZE) return -1;
    if (lazy_sbrk) sz += n;          // demand paging: pages appear on first touch
    else if ((sz = uvmalloc(p->pagetable, sz, sz + n, PTE_R | PTE_W | PTE_X)) == 0) return -1;
  } else if (n < 0) {
    if (sz + n < UBASE) return -1;
    sz = uvmdealloc(p->pagetable, sz, sz + n);
  }
  p->sz = sz;
  return 0;
}

int fork(void) {
  struct proc *p = curproc, *np = allocproc();
  if (!np) return -1;
  if (uvmcopy(p->pagetable, np->pagetable, p->sz) < 0) { np->pagetable = 0; freeproc(np); return -1; }
  np->sz = p->sz;
  np->tf = p->tf;
  np->tf.ksp = (uint64)np->kstack + KSTACKSZ;
  np->tf.x[10] = 0;  // fork returns 0 in the child
  for (int i = 0; i < NOFILE; i++) if (p->ofile[i]) np->ofile[i] = filedup(p->ofile[i]);
  np->cwd = idup(p->cwd);
  safestrcpy(np->name, p->name, sizeof p->name);
  np->parent = p;
  np->prio = p->prio;
  np->state = RUNNABLE;
  return np->pid;
}

void exit(int status) {
  struct proc *p = curproc;
  if (p == initproc) panic("init exiting");
  for (int fd = 0; fd < NOFILE; fd++) if (p->ofile[fd]) { fileclose(p->ofile[fd]); p->ofile[fd] = 0; }
  iput(p->cwd);
  p->cwd = 0;
  for (struct proc *c = proc; c < &proc[NPROC]; c++)
    if (c->parent == p) { c->parent = initproc; if (c->state == ZOMBIE) wakeup(initproc); }
  wakeup(p->parent);
  p->xstate = status;
  p->state = ZOMBIE;
  sched();
  panic("zombie exit");
}

int wait(uint64 addr) {
  struct proc *p = curproc;
  for (;;) {
    int havekids = 0;
    for (struct proc *c = proc; c < &proc[NPROC]; c++) {
      if (c->parent != p) continue;
      havekids = 1;
      if (c->state == ZOMBIE) {
        int pid = c->pid;
        if (addr != 0 && copyout(p->pagetable, addr, (char *)&c->xstate, sizeof c->xstate) < 0) return -1;
        freeproc(c);
        return pid;
      }
    }
    if (!havekids || p->killed) return -1;
    sleep(p);
  }
}

// ---- scheduling --------------------------------------------------------------
static int pick_next(void) {
  for (int lvl = 0; lvl < NLEVELS; lvl++) {
    for (int i = 1; i <= NPROC; i++) {
      int idx = (rr_last[lvl] + i) % NPROC;
      struct proc *p = &proc[idx];
      if (p->state == RUNNABLE && p->prio == lvl) { rr_last[lvl] = idx; return idx; }
    }
  }
  return -1;
}

void scheduler(void) {
  for (;;) {
    intr_off();
    int idx = pick_next();
    if (idx < 0) {          // nothing to run: let interrupts in and wait for one
      intr_on();
      asm volatile("wfi");
      continue;
    }
    struct proc *p = &proc[idx];
    p->state = RUNNING;
    p->nsched++;
    curproc = p;
    w_satp(MAKE_SATP(p->pagetable));
    sfence_vma();
    swtch(&sched_ctx, &p->ctx);
    kvminithart();
    curproc = 0;
  }
}

void sched(void) {
  if (intr_get()) panic("sched: interrupts on");
  if (curproc->state == RUNNING) panic("sched: running");
  swtch(&curproc->ctx, &sched_ctx);
}

int higher_prio_runnable(int level) {
  for (struct proc *p = proc; p < &proc[NPROC]; p++)
    if (p->state == RUNNABLE && p->prio < level) return 1;
  return 0;
}

void yield(void) {
  curproc->state = RUNNABLE;
  sched();
}

void sleep(void *chan) {
  struct proc *p = curproc;
  p->chan = chan;
  p->state = SLEEPING;
  p->used = 0;
  sched();
  p->chan = 0;
}

void wakeup(void *chan) {
  for (struct proc *p = proc; p < &proc[NPROC]; p++)
    if (p != curproc && p->state == SLEEPING && p->chan == chan) p->state = RUNNABLE;
}

int kill(int pid) {
  for (struct proc *p = proc; p < &proc[NPROC]; p++) {
    if (p->pid == pid && p->state != UNUSED) {
      p->killed = 1;
      if (p->state == SLEEPING) p->state = RUNNABLE;
      return 0;
    }
  }
  return -1;
}

void procdump(void) {
  static char *names[] = {"unused", "used", "sleep", "runble", "run", "zombie"};
  printf("\nPID  STATE   PRIO  SCHED  CPUTICKS  NAME   (free pages: %lu, ticks: %lu, cow faults: %lu, sched: %s)\n", kfreepages(), ticks, cow_faults, sched_mlfq ? "MLFQ" : "RR");
  for (struct proc *p = proc; p < &proc[NPROC]; p++) {
    if (p->state == UNUSED) continue;
    printf("%d    %s  %d     %lu     %lu        %s\n", p->pid, names[p->state], p->prio, p->nsched, p->cpu_ticks, p->name);
  }
}
