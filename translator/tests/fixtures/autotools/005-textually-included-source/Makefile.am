bin_PROGRAMS = textual
textual_SOURCES = src/main.c src/table.c

# impl.c is #included BY table.c, never compiled on its own. automake is told
# about it only through EXTRA_DIST, which is a per-directory tarball list --
# it carries no target attribution, which is why the translator has to read
# the #include line instead.
EXTRA_DIST = src/impl.c src/table.h
