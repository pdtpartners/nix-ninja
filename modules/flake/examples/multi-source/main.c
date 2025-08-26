#include <stdio.h>
#include "util.h"

int main() {
  const char* message = msg();
  printf("Hello %s!\n", message);
  return 0;
}
