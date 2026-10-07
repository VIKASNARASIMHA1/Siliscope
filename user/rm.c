#include "user.h"
int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) if (unlink(argv[i]) < 0) { fprintf(2, "rm: %s failed\n", argv[i]); return 1; }
  return 0;
}
