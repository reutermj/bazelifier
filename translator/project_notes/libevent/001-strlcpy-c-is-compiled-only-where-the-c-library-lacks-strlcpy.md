# `strlcpy.c` must be in every library that compiles `evutil.c`, though the ground-truth build never compiled it

**Applies to:** libevent 2.1.12-stable (`libevent.la`, `libevent_core.la`)

The converted module links its sample programs and fails on
`undefined reference: event_strlcpy_`. The obvious reading — the frontend
dropped a source — is half right, and the obvious fix — put `strlcpy.c`
back — is right, but only because of why it was absent.

`Makefile.am` compiles the file conditionally:

```make
if STRLCPY_IMPL
SYS_SRC += strlcpy.c
endif
```

and `STRLCPY_IMPL` is set by configure when the C library has no `strlcpy`.
The machine that produced the ground truth runs glibc 2.39, which has had
`strlcpy` since 2.38, so automake never compiled the file, the command
stream never mentioned it, and the frontend — which reads only what the
build did — never saw it. The hermetic toolchain the module builds with
carries glibc 2.28 headers, so its `HAVE_STRLCPY` probe answers no and every
caller of `evutil.c`'s wrappers expects `event_strlcpy_`, which nothing
defines.

The file guards itself: it compiles to nothing under `EVENT__HAVE_STRLCPY`.
So carrying it unconditionally in every library whose sources include
`evutil.c` (`libevent.la` and `libevent_core.la` — each compiles the core
sources itself rather than depending on the other) is exactly what the
project's build does on a platform without `strlcpy` and a no-op on one
with it. That is the resolution. Do not instead force `HAVE_STRLCPY` to 1,
which would make the build here match the conversion host and break on the
toolchain the module actually targets.

Same shape, same answer, for any other source under an automake
conditional that a configure probe decides: the host's answer picked the
file list, and the module has to carry the file the OTHER answer needs.
