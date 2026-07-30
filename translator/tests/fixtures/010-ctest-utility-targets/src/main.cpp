#include <cstdio>

// Forward-declared rather than pulled from a header: see CMakeLists.txt on
// why calc ships no header (keeps this fixture to the CTest-drop concern).
int calc_answer();

int main() {
    std::printf("answer=%d\n", calc_answer());
    return 0;
}
