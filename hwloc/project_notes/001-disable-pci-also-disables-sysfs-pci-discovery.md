# `--disable-pci` also disables Linux sysfs PCI discovery, and seven of hwloc's own tests with it

**Applies to:** hwloc 2.7.1 (configure options; `tests/hwloc/linux`)

The obvious way to keep a converted hwloc free of libpciaccess is
`--disable-pci`, beside `--disable-libxml2`, `--disable-cairo` and the GPU
runtimes. It is the wrong flag. In `config/hwloc.m4` the same option guards
`HWLOC_HAVE_LINUXPCI` — the Linux backend's discovery of PCI devices from
`/sys`, which needs no library at all — so the module then reports no
`Bridge`/`PCIDev` objects for a recorded sysfs topology, and hwloc's own
suite fails seven of its `tests/hwloc/linux` cases (`*+pci`,
`nvidiagpunumanodes*`, `2pa-pcidomain32bits-disabled`, `32em64t-2n8c+1mic`,
`32intel64-2p8co2t+8ve`). Measured with `make check` under that flag:
upstream fails them too, so a module converted with it can never be green.

The libpciaccess-based `pci` component is separate and is built only where
`pciaccess.h` is found. On a host that has it, the conversion surfaces an
`unconverted_dependency` item for `-lpciaccess`; that is the loud outcome,
not a silent link. So: do not pass `--disable-pci`, and if the pci item
appears, resolve it by deciding about the component, not by adding the flag.

The same shape as `--disable-libxml2`, which is the RIGHT flag for its
purpose: XML support stays through hwloc's own `xml_nolibxml` backend, and
the only thing lost is the dlopen'd `xml_libxml` plugin.
