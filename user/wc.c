#include "user.h"
int main(int argc, char **argv) {
  char buf[512];
  int fd = argc > 1 ? open(argv[1], O_RDONLY) : 0;
  if (fd < 0) { fprintf(2, "wc: cannot open\n"); return 1; }
  int l = 0, w = 0, c = 0, inword = 0, n;
  while ((n = read(fd, buf, sizeof buf)) > 0)
    for (int i = 0; i < n; i++) {
      c++;
      if (buf[i] == '\n') l++;
      if (buf[i] == ' ' || buf[i] == '\n' || buf[i] == '\t') inword = 0;
      else if (!inword) { inword = 1; w++; }
    }
  printf("%d %d %d\n", l, w, c);
  return 0;
}
