"""Instantiates cc_config's shared catalog of common autoconf facts.

A converted CMake project references a catalog probe target
(`@cc_config//catalog:have_endian_h`) rather than declaring its own, so the
probe is analyzed once and runs once per toolchain across the whole build
graph — the sharing property proven by cc_config/sharing_test.sh. See
../../docs/architecture/configure-file-and-toolchain-probes.md.

The catalog is a fixed, hand-maintained set (not generated): the facts here
are the common autoconf checks — the header/symbol/type set that projects
like json-c check. A project needing a fact the catalog lacks is a one-line
addition below. The target name for each is its define lowercased, so the
translator can derive the label from the `#cmakedefine` name it sees.
"""

load("//cc_config:probe.bzl", "check_include_file", "check_symbol_exists", "check_type_size")

def _target_name(define):
    return define.lower()

def cc_config_catalog(name = "catalog", headers = [], symbols = [], types = []):
    """Declares the catalog probe targets.

    Args:
      name: unused — the probe targets are named for their defines, not this.
            Present only because a macro declaring targets conventionally
            takes a `name`; the catalog is declared once so there is nothing
            to disambiguate.
      headers: list of (header, define) for check_include_file.
      symbols: list of (symbol, [headers], define) for check_symbol_exists.
      types:   list of (type, [headers], define) for check_type_size.
    """
    for header, define in headers:
        check_include_file(
            name = _target_name(define),
            header = header,
            define = define,
            visibility = ["//visibility:public"],
        )
    for symbol, symbol_headers, define in symbols:
        check_symbol_exists(
            name = _target_name(define),
            symbol = symbol,
            headers = symbol_headers,
            define = define,
            visibility = ["//visibility:public"],
        )
    for type_name, type_headers, define in types:
        check_type_size(
            name = _target_name(define),
            type = type_name,
            headers = type_headers,
            define = define,
            visibility = ["//visibility:public"],
        )
