#include "kernel.h"

struct devsw devsw[NDEV];
static struct file ftable[NFILE];

void fileinit(void) {}

struct file *filealloc(void) {
  for (struct file *f = ftable; f < ftable + NFILE; f++) if (f->ref == 0) { f->ref = 1; return f; }
  return 0;
}

struct file *filedup(struct file *f) {
  if (f->ref < 1) panic("filedup");
  f->ref++;
  return f;
}

void fileclose(struct file *f) {
  if (f->ref < 1) panic("fileclose");
  if (--f->ref > 0) return;
  struct inode *ip = f->ip;
  struct pipe *pi = f->pipe;
  int type = f->type, wr = f->writable;
  f->type = FD_NONE;
  f->ip = 0;
  f->pipe = 0;
  if (type == FD_PIPE) pipeclose(pi, wr);
  else if (ip) iput(ip);
}

int filestat(struct file *f, uint64 addr) {
  if (f->type != FD_INODE && f->type != FD_DEV) return -1;
  struct stat st;
  ilock(f->ip);
  stati(f->ip, &st);
  iunlock(f->ip);
  return copyout(curproc->pagetable, addr, (char *)&st, sizeof(st));
}

int fileread(struct file *f, uint64 addr, int n) {
  if (!f->readable) return -1;
  if (f->type == FD_PIPE) return piperead(f->pipe, addr, n);
  if (f->type == FD_DEV) {
    if (f->major < 0 || f->major >= NDEV || !devsw[f->major].read) return -1;
    return devsw[f->major].read(1, addr, n);
  }
  ilock(f->ip);
  int r = readi(f->ip, 1, addr, f->off, n);
  if (r > 0) f->off += r;
  iunlock(f->ip);
  return r;
}

int filewrite(struct file *f, uint64 addr, int n) {
  if (!f->writable) return -1;
  if (f->type == FD_PIPE) return pipewrite(f->pipe, addr, n);
  if (f->type == FD_DEV) {
    if (f->major < 0 || f->major >= NDEV || !devsw[f->major].write) return -1;
    return devsw[f->major].write(1, addr, n);
  }
  // write in chunks so one syscall never holds more than a few blocks of state
  int max = 4 * BSIZE, i = 0;
  while (i < n) {
    int n1 = n - i;
    if (n1 > max) n1 = max;
    ilock(f->ip);
    int r = writei(f->ip, 1, addr + i, f->off, n1);
    if (r > 0) f->off += r;
    iunlock(f->ip);
    if (r != n1) break;
    i += r;
  }
  return i == n ? n : -1;
}
