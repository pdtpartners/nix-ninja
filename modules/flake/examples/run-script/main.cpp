#include <cstdio>

extern const char* get_message();

int main() {
    std::printf("%s\n", get_message());
    return 0;
}
