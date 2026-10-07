// System-call dispatch and implementations.
#include "kernel.h"
#include "syscall.h"

extern struct proc proc[];

// ---- argument fetching ---------------------------------------------------------
static uint64 argraw(int n) { return curproc->tf.x[10 + n]; }
static int argint(int n) { return (int)argraw(n); }
static uint64 argaddr(int n) { return argraw(n); }
static int argstr(int n, char *buf, int max) { return copyinstr(curproc->pagetable, buf, argraw(n), max); }

static int fdalloc(struct file *f) {
  for (int fd = 0; fd < NOFILE; fd++) if (!curproc->ofile[fd]) { curproc->ofile[fd] = f; return fd; }
  return -1;
}

static int argfd(int n, int *pfd, struct file **pf) {
  int fd = argint(n);
  if (fd < 0 || fd >= NOFILE || !curproc->ofile[fd]) return -1;
  if (pfd) *pfd = fd;
  if (pf) *pf = curproc->ofile[fd];
  return 0;
}

// ---- process-related -----------------------------------------------------------
static uint64 sys_fork(void) { return fork(); }
static uint64 sys_exit(void) { exit(argint(0)); }
static uint64 sys_wait(void) { return wait(argaddr(0)); }
static uint64 sys_getpid(void) { return curproc->pid; }
static uint64 sys_kill(void) { return kill(argint(0)); }
static uint64 sys_uptime(void) { return ticks; }
static uint64 sys_ps(void) { procdump(); return 0; }

static uint64 sys_sbrk(void) {
  int n = argint(0);
  uint64 addr = curproc->sz;
  if (growproc(n) < 0) return -1;
  return addr;
}

static uint64 sys_sleep(void) {
  int n = argint(0);
  uint64 t0 = ticks;
  while (ticks - t0 < (uint64)n) {
    if (curproc->killed) return -1;
    sleep(&ticks);
  }
  return 0;
}

static uint64 sys_vmctl(void) { return vmctl(argint(0), argint(1)); }
static uint64 sys_freepages(void) { return kfreepages(); }

static uint64 sys_pipe(void) {
  uint64 fdarray = argaddr(0);
  struct file *rf, *wf;
  if (pipealloc(&rf, &wf) < 0) return -1;
  int fd0 = fdalloc(rf), fd1 = -1;
  if (fd0 >= 0) fd1 = fdalloc(wf);
  if (fd0 < 0 || fd1 < 0) {
    if (fd0 >= 0) curproc->ofile[fd0] = 0;
    fileclose(rf); fileclose(wf);
    return -1;
  }
  if (copyout(curproc->pagetable, fdarray, (char *)&fd0, sizeof(int)) < 0 ||
      copyout(curproc->pagetable, fdarray + sizeof(int), (char *)&fd1, sizeof(int)) < 0) {
    curproc->ofile[fd0] = 0; curproc->ofile[fd1] = 0;
    fileclose(rf); fileclose(wf);
    return -1;
  }
  return 0;
}

static uint64 sys_halt(void) {
  printf("\n[kernel] halt: powering off (uptime %lu ticks)\n", ticks);
  *(volatile uint32 *)FINISHER = 0x5555;
  for (;;) {}
}

static uint64 sys_exec(void) {
  char path[MAXPATH], *argv[MAXARG];
  uint64 uargv = argaddr(1);
  int i, ret = -1;
  if (argstr(0, path, MAXPATH) < 0) return -1;
  for (i = 0; i < MAXARG; i++) argv[i] = 0;
  for (i = 0;; i++) {
    uint64 uarg;
    if (i >= MAXARG - 1) goto bad;
    if (copyin(curproc->pagetable, (char *)&uarg, uargv + sizeof(uint64) * i, sizeof(uint64)) < 0) goto bad;
    if (uarg == 0) { argv[i] = 0; break; }
    argv[i] = kalloc();
    if (!argv[i]) goto bad;
    if (copyinstr(curproc->pagetable, argv[i], uarg, PGSIZE) < 0) goto bad;
  }
  ret = exec(path, argv);
bad:
  for (i = 0; i < MAXARG && argv[i]; i++) kfree(argv[i]);
  return ret;
}

// ---- file-related ----------------------------------------------------------------
static uint64 sys_dup(void) {
  struct file *f;
  if (argfd(0, 0, &f) < 0) return -1;
  int fd = fdalloc(f);
  if (fd < 0) return -1;
  filedup(f);
  return fd;
}
static uint64 sys_read(void) {
  struct file *f;
  if (argfd(0, 0, &f) < 0) return -1;
  return fileread(f, argaddr(1), argint(2));
}
static uint64 sys_write(void) {
  struct file *f;
  if (argfd(0, 0, &f) < 0) return -1;
  return filewrite(f, argaddr(1), argint(2));
}
static uint64 sys_close(void) {
  int fd; struct file *f;
  if (argfd(0, &fd, &f) < 0) return -1;
  curproc->ofile[fd] = 0;
  fileclose(f);
  return 0;
}
static uint64 sys_fstat(void) {
  struct file *f;
  if (argfd(0, 0, &f) < 0) return -1;
  return filestat(f, argaddr(1));
}

static struct inode *create(char *path, short type, short major, short minor) {
  char name[DIRSIZ];
  struct inode *dp = nameiparent(path, name), *ip;
  if (!dp) return 0;
  ilock(dp);
  if ((ip = dirlookup(dp, name, 0)) != 0) {
    iunlockput(dp);
    ilock(ip);
    if (type == T_FILE && (ip->type == T_FILE || ip->type == T_DEV)) return ip;
    iunlockput(ip);
    return 0;
  }
  if ((ip = ialloc(dp->dev, type)) == 0) { iunlockput(dp); return 0; }
  ilock(ip);
  ip->major = major; ip->minor = minor; ip->nlink = 1;
  iupdate(ip);
  if (type == T_DIR) {
    dp->nlink++;  // for ".."
    iupdate(dp);
    if (dirlink(ip, ".", ip->inum) < 0 || dirlink(ip, "..", dp->inum) < 0) panic("create dots");
  }
  if (dirlink(dp, name, ip->inum) < 0) panic("create: dirlink");
  iunlockput(dp);
  return ip;
}

static uint64 sys_open(void) {
  char path[MAXPATH];
  int omode = argint(1);
  struct inode *ip;
  if (argstr(0, path, MAXPATH) < 0) return -1;
  if (omode & O_CREATE) {
    if ((ip = create(path, T_FILE, 0, 0)) == 0) return -1;
  } else {
    if ((ip = namei(path)) == 0) return -1;
    ilock(ip);
    if (ip->type == T_DIR && omode != O_RDONLY) { iunlockput(ip); return -1; }
  }
  if (ip->type == T_DEV && (ip->major < 0 || ip->major >= NDEV)) { iunlockput(ip); return -1; }
  struct file *f = filealloc();
  int fd = f ? fdalloc(f) : -1;
  if (!f || fd < 0) {
    if (f) fileclose(f);
    iunlockput(ip);
    return -1;
  }
  if (ip->type == T_DEV) { f->type = FD_DEV; f->major = ip->major; }
  else { f->type = FD_INODE; f->off = 0; }
  f->ip = ip;
  f->readable = !(omode & O_WRONLY);
  f->writable = (omode & O_WRONLY) || (omode & O_RDWR);
  iunlock(ip);
  return fd;
}

static uint64 sys_mkdir(void) {
  char path[MAXPATH];
  struct inode *ip;
  if (argstr(0, path, MAXPATH) < 0 || (ip = create(path, T_DIR, 0, 0)) == 0) return -1;
  iunlockput(ip);
  return 0;
}

static uint64 sys_chdir(void) {
  char path[MAXPATH];
  struct inode *ip;
  if (argstr(0, path, MAXPATH) < 0 || (ip = namei(path)) == 0) return -1;
  ilock(ip);
  if (ip->type != T_DIR) { iunlockput(ip); return -1; }
  iunlock(ip);
  iput(curproc->cwd);
  curproc->cwd = ip;
  return 0;
}

static int isdirempty(struct inode *dp) {
  struct dirent de;
  for (uint off = 2 * sizeof(de); off < dp->size; off += sizeof(de)) {
    if (readi(dp, 0, (uint64)&de, off, sizeof(de)) != sizeof(de)) panic("isdirempty");
    if (de.inum != 0) return 0;
  }
  return 1;
}

static uint64 sys_unlink(void) {
  struct inode *ip, *dp;
  struct dirent de;
  char name[DIRSIZ], path[MAXPATH];
  uint off;
  if (argstr(0, path, MAXPATH) < 0) return -1;
  if ((dp = nameiparent(path, name)) == 0) return -1;
  ilock(dp);
  if ((strcmp(name, ".") == 0 || strcmp(name, "..") == 0)) goto bad;
  if ((ip = dirlookup(dp, name, &off)) == 0) goto bad;
  ilock(ip);
  if (ip->nlink < 1) panic("unlink: nlink < 1");
  if (ip->type == T_DIR && !isdirempty(ip)) { iunlockput(ip); goto bad; }
  memset(&de, 0, sizeof(de));
  if (writei(dp, 0, (uint64)&de, off, sizeof(de)) != sizeof(de)) panic("unlink: writei");
  if (ip->type == T_DIR) { dp->nlink--; iupdate(dp); }
  iunlockput(dp);
  ip->nlink--;
  iupdate(ip);
  iunlockput(ip);
  return 0;
bad:
  iunlockput(dp);
  return -1;
}

static uint64 (*syscalls[])(void) = {
  [SYS_fork] = sys_fork, [SYS_exit] = sys_exit, [SYS_wait] = sys_wait, [SYS_read] = sys_read,
  [SYS_write] = sys_write, [SYS_open] = sys_open, [SYS_close] = sys_close, [SYS_kill] = sys_kill,
  [SYS_exec] = sys_exec, [SYS_fstat] = sys_fstat, [SYS_chdir] = sys_chdir, [SYS_dup] = sys_dup,
  [SYS_getpid] = sys_getpid, [SYS_sbrk] = sys_sbrk, [SYS_sleep] = sys_sleep, [SYS_uptime] = sys_uptime,
  [SYS_unlink] = sys_unlink, [SYS_mkdir] = sys_mkdir, [SYS_halt] = sys_halt, [SYS_ps] = sys_ps,
  [SYS_pipe] = sys_pipe, [SYS_freepages] = sys_freepages, [SYS_vmctl] = sys_vmctl,
};

void syscall(void) {
  struct proc *p = curproc;
  int num = p->tf.x[17];
  if (num > 0 && num < (int)(sizeof(syscalls) / sizeof(syscalls[0])) && syscalls[num])
    p->tf.x[10] = syscalls[num]();
  else {
    printf("%d %s: unknown sys call %d\n", p->pid, p->name, num);
    p->tf.x[10] = -1;
  }
}
