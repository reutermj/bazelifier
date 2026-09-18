# The host's probe answers choose which sources the build compiles

The replicate-behaviour rule in `overview.md` is usually read as being about
VALUES: a config header probed on the consumer's toolchain rather than
copied from the conversion host. libevent showed the same failure one level
up, in the source LIST.

`Makefile.am` says `if STRLCPY_IMPL` / `SYS_SRC += strlcpy.c`, and
configure sets `STRLCPY_IMPL` when the C library has no `strlcpy`. The
conversion host runs glibc 2.39, which has it. So automake never compiled
the file, it never appeared in the command stream, and the frontend — which
reads only what the build did — produced a module without it. The module
builds with the llvm toolchain's glibc 2.28 headers, whose `HAVE_STRLCPY`
probe correctly answers no; every caller then expects `event_strlcpy_`, and
the samples fail to link. Every value in the config header was right. The
file list was the host's.

What the frontend saw was correct and incomplete: the command stream is the
resolved build, and a resolved build has already taken one branch of every
conditional. The information about the branch NOT taken is in `make -p`
(`SYS_SRC = ... $(am__append_N)`, `am__append_N = strlcpy.c`) and in the
conditional's name in `Makefile.in`, neither of which the frontend reads for
sources today. bzl-42r tracks the translator side.

For the agent stage the shape is recognisable and the resolution is safe:
a source under a probe-decided conditional that guards ITSELF against the
probe (`#ifndef EVENT__HAVE_STRLCPY` around the whole file) can be carried
unconditionally, in every library that compiles the sources beside it, and
is a no-op wherever the probe says yes. Do not instead pin the probe to the
host's answer; that fixes the link here and breaks it on the toolchain the
module targets. gnulib's replacement objects (libidn2, gzip) are the same
shape at scale.
