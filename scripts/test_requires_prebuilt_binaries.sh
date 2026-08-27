#!/bin/sh

# The full test suite needs workspace binaries that Cargo does not build as
# dependencies of every test target. A focused TUI test does not need them.
workspace=0
package=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --workspace)
      workspace=1
      ;;
    -p|--package)
      shift
      package=${1:-}
      ;;
    --package=*)
      package=${1#--package=}
      ;;
  esac
  shift
done

if [ "$workspace" -eq 0 ] && [ "$package" = codex-tui ]; then
  exit 1
fi

exit 0
