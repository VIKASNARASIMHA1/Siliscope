#include "user.h"
static char *fmtname(char *path) {
  static char buf[DIRSIZ + 1];
  char *p;
  for (p = path + strlen(path); p >= path && *p != '/'; p--) {}
  p++;
  int n = strlen(p);
  if (n >= DIRSIZ) return p;
  memmove(buf, p, n);
  memset(buf + n, ' ', DIRSIZ - n);
  buf[DIRSIZ] = 0;
  return buf;
}
static void ls(char *path) {
  char buf[128], *p;
  struct stat st;
  struct dirent de;
  int fd = open(path, O_RDONLY);
  if (fd < 0) { fprintf(2, "ls: cannot open %s\n", path); return; }
  if (fstat(fd, &st) < 0) { close(fd); return; }
  if (st.type != T_DIR) {
    printf("%s %d %d %lu\n", fmtname(path), st.type, st.ino, st.size);
  } else {
    strcpy(buf, path);
    p = buf + strlen(buf);
    *p++ = '/';
    while (read(fd, &de, sizeof(de)) == sizeof(de)) {
      if (de.inum == 0) continue;
      memmove(p, de.name, DIRSIZ);
      p[DIRSIZ] = 0;
      struct stat s2;
      int f2 = open(buf, O_RDONLY);
      if (f2 < 0 || fstat(f2, &s2) < 0) { if (f2 >= 0) close(f2); continue; }
      close(f2);
      printf("%s %d %d %lu\n", fmtname(buf), s2.type, s2.ino, s2.size);
    }
  }
  close(fd);
}
int main(int argc, char **argv) {
  if (argc < 2) { ls("."); return 0; }
  for (int i = 1; i < argc; i++) ls(argv[i]);
  return 0;
}
