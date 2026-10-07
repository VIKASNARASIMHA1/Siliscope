// Shared kernel definitions: types, memory map, structures, prototypes.
#ifndef KERNEL_H
#define KERNEL_H

typedef unsigned char uchar;
typedef unsigned char uint8;
typedef unsigned short ushort;
typedef unsigned short uint16;
typedef unsigned int uint;
typedef unsigned int uint32;
typedef unsigned long uint64;
typedef uint64 pte_t;
typedef uint64 *pagetable_t;
#define NULL ((void *)0)

// ---- tunables --------------------------------------------------------------
#define NPROC     32
#define NOFILE    16
#define NFILE     64
#define NINODE    50
#define NBUF      30
#define BSIZE     1024
#define MAXPATH   128
#define MAXARG    16
#define KSTACKSZ  8192
#define NDEV      4
#define CONSOLE   1
#define SCHED_MLFQ_DEFAULT 1   // boot-time default; switch at run time with vmctl(VM_SETSCHED, 0|1)
#ifndef KERNEL_SUPERPAGES
#define KERNEL_SUPERPAGES 1    // 1 = map the kernel's direct map with 2 MiB pages, 0 = 4 KiB pages
#endif
#define NLEVELS   3
#define BOOST_TICKS 50
#ifndef COW_FORK
#define COW_FORK  1     // 1 = copy-on-write fork, 0 = eager copy of every page (for comparison)
#endif
#define PIPESIZE  512

// ---- physical memory map (QEMU "virt" compatible) ---------------------------
#define CLINT          0x02000000L
#define CLINT_MTIMECMP(h) (CLINT + 0x4000 + 8 * (h))
#define CLINT_MTIME    (CLINT + 0xBFF8)
#define PLIC           0x0c000000L
#define UART0          0x10000000L
#define VIRTIO0        0x10001000L
#define UART0_IRQ      10
#define VIRTIO0_IRQ    1
#define FINISHER       0x00100000L
#define KERNBASE       0x80000000L
#define PHYSTOP        (KERNBASE + 128 * 1024 * 1024)

// ---- paging (Sv39) ---------------------------------------------------------
#define PGSIZE 4096
#define PGSHIFT 12
#define PGROUNDUP(a)   (((a) + PGSIZE - 1) & ~((uint64)PGSIZE - 1))
#define PGROUNDDOWN(a) ((a) & ~((uint64)PGSIZE - 1))
#define PTE_V (1L << 0)
#define PTE_R (1L << 1)
#define PTE_W (1L << 2)
#define PTE_X (1L << 3)
#define PTE_U (1L << 4)
#define PTE_A (1L << 6)
#define PTE_D (1L << 7)
#define PTE_COW (1L << 8)   // software bit (RSW): page is shared copy-on-write
#define PTE_SWAP (1L << 9)  // software bit (RSW) on an INVALID pte: page lives in swap, slot in bits 10+
#define SWAPSTART 8192      // first swap block on the disk (right after the file system; see xtask/src/mkfs.rs)
#define SWAPSLOTS 1024      // one slot = one 4 KiB page = 4 blocks
#define MAXRES    1024      // max resident pages the swapper tracks

// vmctl() operations
#define VM_SETLIMIT   0
#define VM_SETPOLICY  1
#define VM_GETSTAT    2
#define VM_RESETSTATS 3
#define VM_SETSCHED   4
#define VM_SETLAZY    5
// swap replacement policies
#define POL_FIFO   0
#define POL_CLOCK  1
#define POL_LRU    2
#define POL_RANDOM 3
// vmctl(VM_GETSTAT, n)
#define ST_LAZY_FAULTS 0
#define ST_SWAP_INS    1
#define ST_SWAP_OUTS   2
#define ST_COW_FAULTS  3
#define ST_RESIDENT    4
#define ST_SWAP_USED   5
#define ST_LIMIT       6
#define PA2PTE(pa) ((((uint64)(pa)) >> 12) << 10)
#define PTE2PA(pte) (((pte) >> 10) << 12)
#define PX(level, va) ((((uint64)(va)) >> (PGSHIFT + 9 * (level))) & 0x1FF)
#define MAKE_SATP(pt) ((8L << 60) | (((uint64)(pt)) >> 12))

// User address space: image at UBASE, heap grows up, stack at the very top.
// (The kernel is identity-mapped, supervisor-only, in every page table.)
#define UBASE         0x40000000L
#define USTACKTOP     0x80000000L
#define USTACKPAGES   8
#define USTACKBASE    (USTACKTOP - USTACKPAGES * PGSIZE)

// ---- RISC-V CSR access ------------------------------------------------------
#define SSTATUS_SPP  (1L << 8)
#define SSTATUS_SPIE (1L << 5)
#define SSTATUS_SIE  (1L << 1)
#define MSTATUS_MPP_MASK (3L << 11)
#define MSTATUS_MPP_S    (1L << 11)
#define MSTATUS_MIE      (1L << 3)
#define SIE_SEIE (1L << 9)
#define SIE_STIE (1L << 5)
#define SIE_SSIE (1L << 1)
#define MIE_MTIE (1L << 7)

#define CSR_R(name) static inline uint64 r_##name(void) { uint64 x; asm volatile("csrr %0, " #name : "=r"(x)); return x; }
#define CSR_W(name) static inline void w_##name(uint64 x) { asm volatile("csrw " #name ", %0" : : "r"(x)); }
CSR_R(mstatus) CSR_W(mstatus) CSR_W(mepc) CSR_R(mie) CSR_W(mie) CSR_W(mscratch) CSR_W(mtvec)
CSR_W(medeleg) CSR_W(mideleg) CSR_W(pmpaddr0) CSR_W(pmpcfg0) CSR_R(mhartid)
CSR_R(sstatus) CSR_W(sstatus) CSR_R(sie) CSR_W(sie) CSR_R(sip) CSR_W(sip) CSR_W(stvec)
CSR_R(sepc) CSR_W(sepc) CSR_R(scause) CSR_R(stval) CSR_W(satp) CSR_R(satp) CSR_W(sscratch)
static inline void sfence_vma(void) { asm volatile("sfence.vma zero, zero"); }
static inline void intr_on(void) { w_sstatus(r_sstatus() | SSTATUS_SIE); }
static inline void intr_off(void) { w_sstatus(r_sstatus() & ~SSTATUS_SIE); }
static inline int intr_get(void) { return (r_sstatus() & SSTATUS_SIE) != 0; }

// ---- processes -------------------------------------------------------------
struct context { uint64 ra, sp, s[12]; };

// Layout is relied upon by trapvec.S: x[i] at 8*i, then the fields below.
struct trapframe {
  uint64 x[32];     // 0..255   (x[0] unused)
  uint64 sepc;      // 256
  uint64 sstatus;   // 264
  uint64 ksp;       // 272  kernel stack top for this process
};

enum procstate { UNUSED, USED, SLEEPING, RUNNABLE, RUNNING, ZOMBIE };

struct proc {
  enum procstate state;
  int pid, killed, xstate;
  void *chan;
  struct proc *parent;
  uint64 sz;                 // end of heap
  pagetable_t pagetable;
  struct trapframe tf;
  struct context ctx;
  char *kstack;
  struct file *ofile[NOFILE];
  struct inode *cwd;
  char name[16];
  int prio, used;            // MLFQ level and ticks used in the current slice
  uint64 cpu_ticks, nsched;  // statistics
};

// ---- file system -----------------------------------------------------------
#define FSMAGIC 0x10203040
#define NDIRECT 12
#define NINDIRECT (BSIZE / sizeof(uint))
#define MAXFILE (NDIRECT + NINDIRECT)
#define ROOTINO 1
#define DIRSIZ 14
#define T_DIR 1
#define T_FILE 2
#define T_DEV 3

struct superblock { uint magic, size, nblocks, ninodes, inodestart, bmapstart; };
struct dinode { short type, major, minor, nlink; uint size; uint addrs[NDIRECT + 1]; };
struct dirent { ushort inum; char name[DIRSIZ]; };
#define IPB (BSIZE / sizeof(struct dinode))
#define IBLOCK(i, sb) ((i) / IPB + (sb).inodestart)
#define BPB (BSIZE * 8)
#define BBLOCK(b, sb) ((b) / BPB + (sb).bmapstart)

struct stat { int dev; uint ino; short type; short nlink; uint64 size; };

struct buf {
  int valid, disk, busy, refcnt;
  uint dev, blockno;
  struct buf *prev, *next;
  uchar data[BSIZE];
};

struct inode {
  uint dev, inum;
  int ref, busy, valid;
  short type, major, minor, nlink;
  uint size;
  uint addrs[NDIRECT + 1];
};

struct pipe;
struct file {
  enum { FD_NONE, FD_INODE, FD_DEV, FD_PIPE } type;
  int ref, readable, writable;
  struct inode *ip;
  struct pipe *pipe;
  uint off;
  short major;
};

struct devsw {
  int (*read)(int user_dst, uint64 dst, int n);
  int (*write)(int user_src, uint64 src, int n);
};
extern struct devsw devsw[];

#define O_RDONLY 0x000
#define O_WRONLY 0x001
#define O_RDWR   0x002
#define O_CREATE 0x200

// ---- prototypes ------------------------------------------------------------
// uart.c / console.c
void uartinit(void); void uartputc_sync(int c); int uartgetc(void); void uartintr(void);
void consoleinit(void); void consoleintr(int c);
// printf.c
void printf(char *fmt, ...); void panic(char *s) __attribute__((noreturn));
// string.c
void *memset(void *, int, uint); int memcmp(const void *, const void *, uint);
void *memmove(void *, const void *, uint); void *memcpy(void *, const void *, uint);
int strlen(const char *); int strcmp(const char *, const char *); int strncmp(const char *, const char *, uint);
char *strncpy(char *, const char *, int); char *safestrcpy(char *, const char *, int);
// kalloc.c / vm.c
void kinit(void); void *kalloc(void); void kfree(void *); uint64 kfreepages(void);
void kvminit(void); void kvminithart(void);
pagetable_t uvmcreate(void);
uint64 uvmalloc(pagetable_t, uint64 oldsz, uint64 newsz, int perm);
uint64 uvmdealloc(pagetable_t, uint64 oldsz, uint64 newsz);
void uvmfree(pagetable_t, uint64 sz);
int uvmcopy(pagetable_t old, pagetable_t new, uint64 sz);
int cowfault(pagetable_t, uint64 va);
extern uint64 cow_faults;
pte_t *walk(pagetable_t, uint64 va, int alloc);
int mappages(pagetable_t, uint64 va, uint64 size, uint64 pa, int perm);
int kpgref(uint64 pa);
// swap.c: demand paging + swapping
uint64 vmfault(pagetable_t, uint64 va);
void swap_free(int slot); void track_remove(pagetable_t, uint64 va); void track_remove_all(pagetable_t);
void swap_resident_range(pagetable_t, uint64 start, uint64 end);
extern int no_evict, memlimit, lazy_sbrk;
uint64 vmctl(int op, int arg);
extern int sched_mlfq; int higher_prio_runnable(int level);
int copyout(pagetable_t, uint64 dstva, char *src, uint64 len);
int copyin(pagetable_t, char *dst, uint64 srcva, uint64 len);
int copyinstr(pagetable_t, char *dst, uint64 srcva, uint64 max);
uint64 walkaddr(pagetable_t, uint64 va);
// proc.c
extern struct proc *curproc;
void procinit(void); void userinit(void); void scheduler(void) __attribute__((noreturn));
int fork(void); void exit(int) __attribute__((noreturn)); int wait(uint64 addr);
void sleep(void *chan); void wakeup(void *chan); void yield(void); int kill(int pid);
int growproc(int n); void procdump(void); void sched(void);
// swtch.S / trapvec.S
void swtch(struct context *, struct context *);
void trapvec(void); void userret(struct trapframe *); void timervec(void);
// trap.c
void trapinit(void); void usertrapret(void);
extern uint64 ticks;
// plic.c
void plicinit(void); void plicinithart(void); int plic_claim(void); void plic_complete(int irq);
// virtio.c
uint64 virtio_disk_sectors(void);
void virtio_disk_init(void); void virtio_disk_rw(struct buf *, int write); void virtio_disk_intr(void);
// bio.c
void binit(void); struct buf *bread(uint dev, uint blockno); void bwrite(struct buf *); void brelse(struct buf *);
void bwrite_new(uint dev, uint blockno, const void *data);
// fs.c
void fsinit(int dev);
struct inode *namei(char *path); struct inode *nameiparent(char *path, char *name);
struct inode *ialloc(uint dev, short type); struct inode *idup(struct inode *);
void ilock(struct inode *); void iunlock(struct inode *); void iput(struct inode *);
void iunlockput(struct inode *); void iupdate(struct inode *);
int readi(struct inode *, int user_dst, uint64 dst, uint off, uint n);
int writei(struct inode *, int user_src, uint64 src, uint off, uint n);
int dirlink(struct inode *, char *name, uint inum); struct inode *dirlookup(struct inode *, char *name, uint *poff);
void stati(struct inode *, struct stat *);
// file.c
void fileinit(void); struct file *filealloc(void); struct file *filedup(struct file *); void fileclose(struct file *);
int fileread(struct file *, uint64 addr, int n); int filewrite(struct file *, uint64 addr, int n);
int filestat(struct file *, uint64 addr);
// pipe.c
int pipealloc(struct file **, struct file **); void pipeclose(struct pipe *, int writable);
int pipewrite(struct pipe *, uint64 addr, int n); int piperead(struct pipe *, uint64 addr, int n);
// exec.c / syscall.c
int exec(char *path, char **argv);
void syscall(void);

#endif
