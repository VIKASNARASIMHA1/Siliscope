// In-guest regression suite: exercises fork/wait/exec, address-space isolation,
// sbrk, the file system (incl. indirect blocks), blocking sleep, kill and
// memory protection. Prints "ktest: ALL PASSED" on success.
#include "user.h"

static int failures;
#define CHECK(c, msg) do { if (!(c)) { printf("  FAIL: %s (line %d)\n", msg, __LINE__); failures++; } } while (0)

static int g = 1;

static void test_fork_wait(void) {
  printf("ktest: fork/wait ... ");
  int seen = 0, st;
  for (int i = 0; i < 8; i++) {
    int pid = fork();
    if (pid == 0) exit(i + 10);
    CHECK(pid > 0, "fork");
  }
  for (int i = 0; i < 8; i++) {
    int p = wait(&st);
    CHECK(p > 0, "wait returned a pid");
    CHECK(st >= 10 && st < 18, "exit status propagated");
    if (st >= 10 && st < 18) seen |= 1 << (st - 10);
  }
  CHECK(seen == 0xff, "all 8 children reaped exactly once");
  CHECK(wait(&st) == -1, "wait with no children fails");
  printf("done\n");
}

static void test_exec(void) {
  printf("ktest: exec ... ");
  int pid = fork(), st = -1;
  if (pid == 0) { char *a[] = {"echo", "[exec ok]", 0}; exec("echo", a); exit(99); }
  wait(&st);
  CHECK(st == 0, "child ran /echo and exited 0");
  char *bad[] = {"nonexistent", 0};
  CHECK(exec("nonexistent", bad) == -1, "exec of missing file fails");
}

static void test_isolation(void) {
  printf("ktest: address-space isolation ... ");
  int st = -1;
  if (fork() == 0) { g = 99; exit(g == 99 ? 0 : 1); }
  wait(&st);
  CHECK(st == 0, "child wrote its own copy");
  CHECK(g == 1, "parent's memory unchanged after child write");
  printf("done\n");
}

static void test_sbrk(void) {
  printf("ktest: sbrk ... ");
  char *base = sbrk(0);
  char *p = sbrk(1 << 20);
  CHECK(p == base, "sbrk returns old break");
  int ok = 1;
  for (int i = 0; i < (1 << 20); i += 64) p[i] = (char)(i / 64);
  for (int i = 0; i < (1 << 20); i += 64) if (p[i] != (char)(i / 64)) ok = 0;
  CHECK(ok, "1 MiB heap readable and writable");
  sbrk(-(1 << 20));
  CHECK(sbrk(0) == base, "heap shrinks back");
  printf("done\n");
}

static void test_fs(void) {
  printf("ktest: file system ... ");
  struct stat st;
  CHECK(mkdir("/kt") == 0, "mkdir");
  CHECK(mkdir("/kt/sub") == 0, "nested mkdir");
  int fd = open("/kt/big", O_CREATE | O_RDWR);
  CHECK(fd >= 0, "create file");
  char buf[1000];
  for (int blk = 0; blk < 40; blk++) {          // 40,000 bytes -> needs the indirect block
    for (int i = 0; i < 1000; i++) buf[i] = (char)(blk * 7 + i);
    CHECK(write(fd, buf, 1000) == 1000, "write chunk");
  }
  close(fd);
  fd = open("/kt/big", O_RDONLY);
  CHECK(fd >= 0 && fstat(fd, &st) == 0 && st.size == 40000, "file size is 40000");
  int bad = 0;
  for (int blk = 0; blk < 40; blk++) {
    CHECK(read(fd, buf, 1000) == 1000, "read chunk");
    for (int i = 0; i < 1000; i++) if (buf[i] != (char)(blk * 7 + i)) bad++;
  }
  CHECK(bad == 0, "data read back matches");
  CHECK(read(fd, buf, 10) == 0, "EOF at end of file");
  close(fd);
  // dup shares the file offset
  fd = open("/kt/dup", O_CREATE | O_RDWR);
  int fd2 = dup(fd);
  write(fd, "ab", 2); write(fd2, "cd", 2);
  close(fd); close(fd2);
  fd = open("/kt/dup", O_RDONLY);
  char r[8] = {0};
  CHECK(read(fd, r, 4) == 4 && r[0] == 'a' && r[1] == 'b' && r[2] == 'c' && r[3] == 'd', "dup shares offset");
  close(fd);
  // directory listing
  fd = open("/kt", O_RDONLY);
  struct dirent de;
  int n = 0;
  while (read(fd, &de, sizeof de) == sizeof de) if (de.inum) n++;
  close(fd);
  CHECK(n == 5, "directory has ., .., big, dup, sub");
  CHECK(unlink("/kt") == -1, "cannot unlink non-empty directory");
  CHECK(unlink("/kt/big") == 0 && unlink("/kt/dup") == 0 && unlink("/kt/sub") == 0, "unlink files");
  CHECK(open("/kt/big", O_RDONLY) < 0, "unlinked file is gone");
  CHECK(unlink("/kt") == 0, "unlink empty directory");
  printf("done\n");
}

static void test_sleep(void) {
  printf("ktest: sleep/uptime ... ");
  int t0 = uptime();
  sleep(3);
  CHECK(uptime() - t0 >= 3, "sleep(3) lasted at least 3 ticks");
  printf("done\n");
}


static void test_cow(void) {
  printf("ktest: copy-on-write fork ... ");
  int npages = 64;
  char *h = sbrk(npages * 4096);
  for (int i = 0; i < npages; i++) h[i * 4096] = (char)(i + 1);
  int free0 = freepages();
  int pid = fork();
  if (pid == 0) {                       // child: modifies page 3, checks every other page is intact
    int ok = (freepages() >= free0 - 12);          // fork itself copied (almost) nothing
    h[3 * 4096] = 99;
    for (int i = 0; i < npages; i++) if (i != 3 && h[i * 4096] != (char)(i + 1)) ok = 0;
    if (h[3 * 4096] != 99) ok = 0;
    exit(ok ? 0 : 1);
  }
  int st = -1;
  wait(&st);
  CHECK(st == 0, "child saw its own write, other pages intact, and fork did not copy memory");
  CHECK(h[3 * 4096] == 4, "parent's page unaffected by child's write");
  h[5 * 4096] = 77;                     // parent write after the child is gone (sole owner: no copy needed)
  CHECK(h[5 * 4096] == 77, "parent can still write");
  int used = free0 - freepages();
  CHECK(used >= -1 && used <= 1, "all shared/copied pages were released after the child exited");
  // kernel-side writes into a shared page (read() into a COW buffer) must also un-share it
  char local[64];
  for (int i = 0; i < 64; i++) local[i] = 1;
  int pfd[2];
  CHECK(pipe(pfd) == 0, "pipe for COW test");
  write(pfd[1], "kernel-wrote-this", 17);
  pid = fork();
  if (pid == 0) {
    char tmp[32];
    int n = read(pfd[0], tmp, 17);                // kernel copyout into the child's stack
    int ok = (n == 17 && tmp[0] == 'k');
    for (int i = 0; i < 64; i++) if (local[i] != 1) ok = 0;
    exit(ok ? 0 : 1);
  }
  wait(&st);
  CHECK(st == 0, "copyout into a COW stack page works in the child");
  close(pfd[0]); close(pfd[1]);
  sbrk(-(npages * 4096));
  printf("done\n");
}

static void test_pipe(void) {
  printf("ktest: pipes ... ");
  int p[2], st = -1;
  CHECK(pipe(p) == 0, "pipe()");
  if (fork() == 0) {                    // writer: 3000 bytes through a 512-byte pipe (must block and resume)
    close(p[0]);
    char b[100];
    for (int blk = 0; blk < 30; blk++) {
      for (int i = 0; i < 100; i++) b[i] = (char)(blk + i);
      if (write(p[1], b, 100) != 100) exit(1);
    }
    close(p[1]);
    exit(0);
  }
  close(p[1]);
  char buf[128];
  int total = 0, bad = 0, n;
  while ((n = read(p[0], buf, sizeof buf)) > 0) {
    for (int i = 0; i < n; i++, total++) if (buf[i] != (char)(total / 100 + total % 100)) bad++;
  }
  CHECK(total == 3000, "reader received all 3000 bytes");
  CHECK(bad == 0, "bytes arrived in order and intact");
  CHECK(n == 0, "EOF after the last writer closed");
  close(p[0]);
  wait(&st);
  CHECK(st == 0, "writer finished");
  // write to a pipe with no readers must fail instead of blocking forever
  CHECK(pipe(p) == 0, "second pipe");
  close(p[0]);
  CHECK(write(p[1], "x", 1) == -1, "write with no reader fails");
  close(p[1]);
  // stdout of an exec'd program redirected into a pipe
  CHECK(pipe(p) == 0, "third pipe");
  if (fork() == 0) {
    close(1); dup(p[1]); close(p[0]); close(p[1]);
    char *a[] = {"echo", "pipe-ok", 0};
    exec("echo", a);
    exit(1);
  }
  close(p[1]);
  char out[32] = {0};
  n = read(p[0], out, 31);
  CHECK(n == 8 && !memcmp(out, "pipe-ok\n", 8), "child's stdout captured through the pipe");
  close(p[0]);
  wait(&st);
  printf("done\n");
}

static void test_lazy(void) {
  printf("ktest: lazy sbrk (demand paging) ... ");
  long f0 = freepages();
  int pages = 256;
  char *p = sbrk(pages * 4096);
  long after_sbrk = f0 - freepages();
  CHECK(after_sbrk <= 2, "sbrk(1 MiB) allocates no frames up front");
  CHECK(p[100 * 4096] == 0, "untouched lazy page reads as zero");
  for (int i = 0; i < 10; i++) p[i * 4096 * 7] = (char)(i + 1);
  long used = f0 - freepages();
  CHECK(used >= 10 && used <= 20, "only touched pages (plus page tables) consume frames");
  int ok = 1;
  for (int i = 0; i < 10; i++) if (p[i * 4096 * 7] != (char)(i + 1)) ok = 0;
  CHECK(ok, "touched pages keep their data");
  int fd = open("/README.txt", O_RDONLY);       // kernel copyout into a never-touched lazy page
  CHECK(fd >= 0 && read(fd, p + 200 * 4096, 20) == 20 && p[200 * 4096] == 'W', "read() into an untouched lazy page works");
  close(fd);
  sbrk(-(pages * 4096));
  CHECK(freepages() - f0 >= -2 && freepages() - f0 <= 2, "all frames returned after shrinking");
  printf("done\n");
}

static void test_swap(void) {
  printf("ktest: swapping (all 4 policies + fork) ... ");
  int ws = 24, limit = 8, st;
  for (int pol = 0; pol < 4; pol++) {
    char *b = sbrk(ws * 4096);
    vmctl(VM_RESETSTATS, 0);
    vmctl(VM_SETPOLICY, pol);
    if (vmctl(VM_SETLIMIT, limit) != limit) { CHECK(0, "swap area available on the disk image"); sbrk(-(ws * 4096)); printf("skipped\n"); return; }
    for (int i = 0; i < ws; i++) for (int j = 0; j < 4096; j += 512) b[i * 4096 + j] = (char)(i * 3 + j / 512);
    int bad = 0;
    for (int round = 0; round < 3; round++)
      for (int i = ws - 1; i >= 0; i--) for (int j = 0; j < 4096; j += 512) if (b[i * 4096 + j] != (char)(i * 3 + j / 512)) bad++;
    CHECK(bad == 0, "data intact after being swapped out and back in");
    CHECK(vmctl(VM_GETSTAT, ST_SWAP_OUTS) > 0 && vmctl(VM_GETSTAT, ST_SWAP_INS) > 0, "pages really were swapped");
    CHECK(vmctl(VM_GETSTAT, ST_RESIDENT) <= limit, "resident set respects the limit");
    if (pol == 2) {                                   // fork while most pages are in swap
      int pid = fork();
      if (pid == 0) {
        int ok = 1;
        for (int i = 0; i < ws; i++) if (b[i * 4096] != (char)(i * 3)) ok = 0;
        b[5 * 4096] = 99;                             // child's write must not leak to the parent
        exit(ok ? 0 : 1);
      }
      wait(&st);
      CHECK(st == 0, "child sees all of the parent's pages (swapped ones included)");
      CHECK(b[5 * 4096] == (char)(5 * 3), "parent unaffected by child write");
    }
    sbrk(-(ws * 4096));
    vmctl(VM_SETLIMIT, 0);
    CHECK(vmctl(VM_GETSTAT, ST_SWAP_USED) == 0, "swap slots released");
    CHECK(vmctl(VM_GETSTAT, ST_RESIDENT) == 0, "tracking list emptied");
  }
  printf("done\n");
}

static void test_protection(void) {
  printf("ktest: memory protection ... ");
  int st = 0;
  if (fork() == 0) { volatile int *k = (int *)0x80000000L; int v = *k; printf("BUG: read kernel memory %d\n", v); exit(0); }
  wait(&st);
  CHECK(st == -1, "reading kernel memory kills the process");
  st = 0;
  if (fork() == 0) { volatile int *z = (int *)0; *z = 1; exit(0); }
  wait(&st);
  CHECK(st == -1, "writing unmapped address 0 kills the process");
  printf("done\n");
}

static void test_kill(void) {
  printf("ktest: kill ... ");
  int pid = fork(), st = 0;
  if (pid == 0) for (;;) {}
  sleep(2);
  CHECK(kill(pid) == 0, "kill");
  wait(&st);
  CHECK(st == -1, "killed process reports -1");
  printf("done\n");
}

static void test_fork_limit(void) {
  printf("ktest: process table limit ... ");
  int n = 0;
  for (;;) {
    int pid = fork();
    if (pid == 0) { sleep(3); exit(0); }
    if (pid < 0) break;
    n++;
  }
  CHECK(n >= 10 && n < 40, "fork eventually fails gracefully");
  int st;
  for (int i = 0; i < n; i++) wait(&st);
  int again = fork();
  if (again == 0) exit(0);
  CHECK(again >= 0, "fork works again after reaping");
  wait(&st);
  printf("done (%d children)\n", n);
}

int main(void) {
  printf("ktest: starting\n");
  test_fork_wait();
  test_exec();
  test_isolation();
  test_sbrk();
  test_fs();
  test_cow();
  test_pipe();
  test_lazy();
  test_swap();
  test_sleep();
  test_protection();
  test_kill();
  test_fork_limit();
  if (failures == 0) printf("ktest: ALL PASSED\n");
  else printf("ktest: %d FAILED\n", failures);
  return failures != 0;
}
