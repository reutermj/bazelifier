#define CAT_(a, b) a##b
#define CAT(a, b) CAT_(a, b)

/* Named by WHICH, so each archive defines its own function; reports whether
   it was compiled with SSE4.1, which only libone's -msse4.1 turns on. */
int CAT(variant_, WHICH)(void) {
#ifdef __SSE4_1__
    return WHICH * 10 + 1;
#else
    return WHICH * 10;
#endif
}
