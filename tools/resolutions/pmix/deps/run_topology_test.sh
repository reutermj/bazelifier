#!/usr/bin/env bash
# hwloc's tests/hwloc/{linux,linux/allowed,x86,x86+linux,xml}/test-topology.sh
# re-expressed as one Bazel-runnable runner: run lstopo-no-graphics over a
# recorded topology (a sysfs or cpuid tarball, a synthetic description, or an
# XML file) and diff what it prints against the checked-in expectation. The
# upstream drivers are configure-generated (.sh.in, with @HWLOC_top_builddir@
# and the build layout hard-coded), so the CHECK is reproduced here rather
# than the scripts; every environment variable each driver exports is set
# here, per kind, and the .options/.env/.source/.exclude sidecars are read
# the same way, beside the entry.
#
#   run_topology_test.sh <lstopo> <kind> <entry>
#   kind:  linux | allowed | x86 | x86+linux | xml
#   entry: the automake TESTS entry — <name>.output, or <name>.xml for the
#          xml kind's round-trip tests
set -u
lstopo="$1"; kind="$2"; entry="$3"
dir="$(dirname "$entry")"

export HWLOC_DEBUG_CHECK=1 LANG=C LC_ALL=C
# No plugin is built in this module (libxml2 is disabled by decision, see
# BUILD.bazel), so the search path points at nothing rather than a build tree.
export HWLOC_PLUGINS_PATH=/nonexistent

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
strip_gp() { sed -e 's/ gp_index="[0-9]*"//'; }

case "$kind" in
  xml)
    export HWLOC_LIBXML_CLEANUP=1
    base="$(basename "$entry" .xml)"; base="$(basename "$base" .output)"
    source="$dir/$base.xml"
    [ -f "$dir/$base.source" ] && source="$dir/$(cat "$dir/$base.source")"
    opts=""; [ -f "$dir/$base.options" ] && opts="$(cat "$dir/$base.options")"
    [ -f "$dir/$base.env" ] && . "$dir/$base.env"
    case "$entry" in
      *.xml)  # round trip: export the topology as XML and expect the input back
        "$lstopo" --if xml --input "$source" --of xml "$tmp/out.xml" $opts || exit 1
        diff -u -w "$entry" "$tmp/out.xml" ;;
      *)      # a console rendering of the XML topology
        "$lstopo" --if xml --input "$source" "$tmp/lstopo_xml.output" $opts || exit 1
        diff -u -w "$entry" "$tmp/lstopo_xml.output" ;;
    esac
    ;;
  linux|x86|x86+linux|allowed)
    export HWLOC_DONT_ADD_VERSION_INFO=1 HWLOC_XML_EXPORT_SUPPORT=0 HWLOC_DEBUG_SORT_CHILDREN=1
    topology="${entry%.output}"
    case "$kind" in
      allowed) source="$topology.fsroot.tar.bz2"
               [ -f "$topology.source" ] && source="$dir/$(cat "$topology.fsroot.source")" ;;
      *)       source="$topology.tar.bz2"
               [ -f "$topology.source" ] && source="$dir/$(cat "$topology.source")" ;;
    esac
    [ "$kind" = x86+linux ] && export HWLOC_COMPONENTS=x86,linux,stop
    [ -f "$topology.env" ] && . "$topology.env"
    tar_opts=""
    [ -f "$dir/$(basename "$topology").exclude" ] && tar_opts="--exclude-from=$dir/$(basename "$topology").exclude"
    if ! ( bunzip2 -c "$source" | ( cd "$tmp" && tar xf - $tar_opts ) ); then
      echo "failed to extract $source"; exit 1
    fi
    actual="$(echo "$tmp"/*)"
    opts=""; [ -r "$topology.options" ] && opts="$(cat "$topology.options")"
    case "$kind" in
      linux)
        [ -n "$opts" ] || opts="-v -"
        export HWLOC_COMPONENTS=linux,stop HWLOC_FSROOT="$actual" HWLOC_DUMPED_HWDATA_DIR=/var/run/hwloc
        "$lstopo" $opts | strip_gp > "$tmp/out" || exit 1
        diff -u -w "$entry" "$tmp/out" ;;
      allowed)
        [ -n "$opts" ] || opts="-v -"
        # The driver tells lstopo the synthetic topology IS this system and
        # to apply the fsroot's allowed-resource masks to it.
        export HWLOC_THISSYSTEM=1 HWLOC_THISSYSTEM_ALLOWED_RESOURCES=1
        HWLOC_FSROOT="$actual" "$lstopo" --if synthetic -i "$(cat "$topology.synthetic")" $opts > "$tmp/out" || exit 1
        diff -u -w "$entry" "$tmp/out" ;;
      x86)
        [ -n "$opts" ] || opts="--of xml -"
        export HWLOC_THISSYSTEM=0 HWLOC_X86_TOPOEXT_NUMANODES=1
        "$lstopo" -i "$actual" --if cpuid $opts | strip_gp > "$tmp/out" || exit 1
        diff -b -u "$entry" "$tmp/out" ;;
      x86+linux)
        [ -n "$opts" ] || opts="--of xml -"
        export HWLOC_THISSYSTEM=0 HWLOC_FSROOT="$actual/fsroot" HWLOC_CPUID_PATH="$actual/cpuid"
        "$lstopo" $opts | strip_gp > "$tmp/out" || exit 1
        diff -b -u "$entry" "$tmp/out" ;;
    esac
    ;;
  *) echo "unknown kind $kind"; exit 2 ;;
esac
