#ifndef USER_H
#define USER_H
typedef unsigned char uchar; typedef unsigned short ushort; typedef unsigned int uint; typedef unsigned long uint64;
#define NULL ((void *)0)
#define O_RDONLY 0x000
#define O_WRONLY 0x001
#define O_RDWR   0x002
#define O_CREATE 0x200
#define T_DIR 1
#define T_FILE 2
#define T_DEV 3
#define DIRSIZ 14
struct stat { int dev; uint ino; short type; short nlink; uint64 size; };
struct dirent { ushort inum; char name[DIRSIZ]; };

// system calls
int fork(void); int exit(int) __attribute__((noreturn)); int wait(int *);
int read(int, void *, int); int write(int, const void *, int); int open(const char *, int); int close(int);
int kill(int); int exec(const char *, char **); int fstat(int, struct stat *); int chdir(const char *);
int dup(int); int getpid(void); char *sbrk(int); int sleep(int); int uptime(void);
int unlink(const char *); int mkdir(const char *); int halt(void) __attribute__((noreturn)); int ps(void);
int pipe(int *); int freepages(void); long vmctl(int op, int arg);
#define VM_SETLIMIT 0
#define VM_SETPOLICY 1
#define VM_GETSTAT 2
#define VM_RESETSTATS 3
#define VM_SETSCHED 4
#define VM_SETLAZY 5
#define POL_FIFO 0
#define POL_CLOCK 1
#define POL_LRU 2
#define POL_RANDOM 3
#define ST_LAZY_FAULTS 0
#define ST_SWAP_INS 1
#define ST_SWAP_OUTS 2
#define ST_COW_FAULTS 3
#define ST_RESIDENT 4
#define ST_SWAP_USED 5

// library
int strlen(const char *); int strcmp(const char *, const char *); char *strcpy(char *, const char *);
void *memset(void *, int, uint); void *memmove(void *, const void *, uint); void *memcpy(void *, const void *, uint);
int memcmp(const void *, const void *, uint); char *strchr(const char *, char); char *gets(char *, int);
int atoi(const char *); void printf(const char *fmt, ...);   // printf writes to fd 1
void fprintf(int fd, const char *fmt, ...);
#endif
