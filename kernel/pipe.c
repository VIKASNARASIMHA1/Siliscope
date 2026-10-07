// Pipes: a 512-byte ring buffer shared by a read file and a write file.
// Writers block when it is full, readers block when it is empty; closing the
// last writer gives readers EOF, closing the last reader makes writes fail.
#include "kernel.h"

struct pipe {
  char data[PIPESIZE];
  uint nread, nwrite;     // total bytes ever read / written
  int readopen, writeopen;
};

int pipealloc(struct file **f0, struct file **f1) {
  struct pipe *pi = 0;
  *f0 = *f1 = 0;
  if ((*f0 = filealloc()) == 0 || (*f1 = filealloc()) == 0) goto bad;
  if ((pi = (struct pipe *)kalloc()) == 0) goto bad;
  pi->readopen = pi->writeopen = 1;
  pi->nread = pi->nwrite = 0;
  (*f0)->type = FD_PIPE; (*f0)->readable = 1; (*f0)->writable = 0; (*f0)->pipe = pi;
  (*f1)->type = FD_PIPE; (*f1)->readable = 0; (*f1)->writable = 1; (*f1)->pipe = pi;
  return 0;
bad:
  if (pi) kfree(pi);
  if (*f0) fileclose(*f0);
  if (*f1) fileclose(*f1);
  return -1;
}

void pipeclose(struct pipe *pi, int writable) {
  if (writable) { pi->writeopen = 0; wakeup(&pi->nread); }
  else { pi->readopen = 0; wakeup(&pi->nwrite); }
  if (!pi->readopen && !pi->writeopen) kfree(pi);
}

int pipewrite(struct pipe *pi, uint64 addr, int n) {
  int i = 0;
  while (i < n) {
    if (!pi->readopen || curproc->killed) return -1;
    if (pi->nwrite == pi->nread + PIPESIZE) {      // full: let readers run
      wakeup(&pi->nread);
      sleep(&pi->nwrite);
    } else {
      char ch;
      if (copyin(curproc->pagetable, &ch, addr + i, 1) < 0) break;
      pi->data[pi->nwrite++ % PIPESIZE] = ch;
      i++;
    }
  }
  wakeup(&pi->nread);
  return i;
}

int piperead(struct pipe *pi, uint64 addr, int n) {
  while (pi->nread == pi->nwrite && pi->writeopen) {   // empty: wait for data or EOF
    if (curproc->killed) return -1;
    sleep(&pi->nread);
  }
  int i;
  for (i = 0; i < n; i++) {
    if (pi->nread == pi->nwrite) break;
    char ch = pi->data[pi->nread++ % PIPESIZE];
    if (copyout(curproc->pagetable, addr + i, &ch, 1) < 0) break;
  }
  wakeup(&pi->nwrite);
  return i;
}
