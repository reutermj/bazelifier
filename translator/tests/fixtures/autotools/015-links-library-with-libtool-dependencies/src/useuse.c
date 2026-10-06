/* 013 installs no header; its entry point, as it defines it. */
int usegreet_value(void);

int useuse_value(void) {
    return usegreet_value() + 1;
}
