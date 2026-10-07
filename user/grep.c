// grep PATTERN [file]: print lines containing PATTERN (plain substring match).
#include "user.h"

static int match_here(const char *s, const char *p) {
  while (*p) { if (*s != *p) return 0; s++; p++; }
  return 1;
}
static int contains(const char *hay, const char *needle) {
  for (; *hay; hay++) if (match_here(hay, needle)) return 1;
  return *needle == 0;
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(2, "usage: grep pattern [file]\n"); return 2; }
  int fd = 0;
  if (argc > 2 && (fd = open(argv[2], O_RDONLY)) < 0) { fprintf(2, "grep: cannot open %s\n", argv[2]); return 2; }
  char line[512], buf[256];
  int len = 0, n, found = 0;
  while ((n = read(fd, buf, sizeof buf)) > 0) {
    for (int i = 0; i < n; i++) {
      if (buf[i] == '\n' || len == (int)sizeof(line) - 2) {
        line[len] = 0;
        if (contains(line, argv[1])) { printf("%s\n", line); found = 1; }
        len = 0;
      } else line[len++] = buf[i];
    }
  }
  if (len > 0) { line[len] = 0; if (contains(line, argv[1])) { printf("%s\n", line); found = 1; } }
  return found ? 0 : 1;
}
