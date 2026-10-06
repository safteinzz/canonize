#!/usr/bin/env bash
# Render the README assets in a container, so a machine needs podman or docker
# and nothing else: no vhs, no font, and the same frames on every machine that
# runs it.
#
#   ./render.sh              every tape
#   ./render.sh setup        one tape
#
# Each tape runs in the image from ./Dockerfile, in a container of its own that
# stages, records and tears down, because every tape changes the stage it runs
# in (a fix, an adopt, a move) and the next one has to start from it untouched.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

ENGINE="$(command -v podman || command -v docker || true)"
[ -n "$ENGINE" ] || { echo "render.sh needs podman or docker" >&2; exit 1; }
IMAGE=localhost/canonize-render

TAPES=("$@")
[ ${#TAPES[@]} -gt 0 ] || TAPES=(setup agents skills projects shots move)

(cd .. && cargo build --release)
# The Dockerfile is the whole build context: nothing in this folder is copied in.
"$ENGINE" build -q -t "$IMAGE" - < Dockerfile > /dev/null

# Run as this user, not root: the images it writes stay yours, and the staged
# shell's prompt ends in `$` as it does on a machine rendering without a
# container, rather than root's `#`.
if [ "$(basename "$ENGINE")" = docker ]; then
  USER_ARGS=(--user "$(id -u):$(id -g)" -e HOME=/tmp)
else
  USER_ARGS=(--userns=keep-id -e HOME=/tmp)
fi

# No network: the stage invents everything it shows, so nothing in a take has a
# reason to leave the machine.
for t in "${TAPES[@]}"; do
  echo "── $t.tape"
  "$ENGINE" run --rm --network none "${USER_ARGS[@]}" \
    -v "$(cd .. && pwd):/work/canonize:Z" -w /work/canonize/demo \
    --entrypoint bash "$IMAGE" \
    -c "vhs $t.tape > /dev/null; s=\$?; ./stage.sh down > /dev/null; exit \$s"
done
