#include "user.h"
int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) if (mkdir(argv[i]) < 0) { fprintf(2, "mkdir: %s failed\n", argv[i]); return 1; }
  return 0;
}
