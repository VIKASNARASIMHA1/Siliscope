#include "user.h"
int main(int argc, char **argv) {
  char buf[512];
  int fd = 0;
  for (int i = 1; i < argc || i == 1; i++) {
    if (argc > 1) { fd = open(argv[i], O_RDONLY); if (fd < 0) { fprintf(2, "cat: cannot open %s\n", argv[i]); return 1; } }
    int n;
    while ((n = read(fd, buf, sizeof buf)) > 0) write(1, buf, n);
    if (argc > 1) close(fd); else break;
  }
  return 0;
}
