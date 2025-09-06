#include <stdio.h>

// Forward declaration for the function in generated.c
const char* get_example_name();

int main() {
    printf("Hello %s!\n", get_example_name());
    return 0;
}