#!/usr/bin/env bash
# Our SDL tree (docs/sdl-vendoring.md): upstream at a pinned release in third_party/SDL (gitignored)
# plus our changes, kept as one patch (patches/sdl-patchbox.patch). Cargo builds it through
# third_party/sdl3-src; this script only provisions and records the tree.
#
#   scripts/sdl.sh fetch    clone the pinned release (network, once), check the commit, apply the patch
#   scripts/sdl.sh apply    apply the patch to a pristine tree (a tree with changes is left alone)
#   scripts/sdl.sh regen    rewrite the patch from the tree, new files included; then commit the patch
#   scripts/sdl.sh status   tag, commit and local changes of the tree
#
# The tree is the source of truth while working on SDL: edit it, build (cargo rebuilds SDL), then
# `regen`. A bare `git diff` would drop new files: regen intent-to-adds them first.
#
# Upgrading SDL: change SDL_TAG and SDL_COMMIT (and the version in third_party/sdl3-src: it must
# equal what sdl3-sys pins), move the old tree away, `fetch`, resolve what the patch no longer
# applies to (`git apply --3way`), build, test, `regen`.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDL_URL=https://github.com/libsdl-org/SDL.git
SDL_TAG=release-3.4.18
SDL_COMMIT=829a65d769d935c4852f8159e964312c0957260a
TREE="$REPO/third_party/SDL"
PATCH="$REPO/patches/sdl-patchbox.patch"

pristine() {
  git -C "$TREE" diff --quiet HEAD -- . && git -C "$TREE" diff --cached --quiet -- . \
    && [ -z "$(git -C "$TREE" ls-files --others --exclude-standard)" ]
}

apply() {
  if [ ! -f "$PATCH" ]; then
    echo ">> no $PATCH yet: the tree stays upstream"
  elif pristine; then
    git -C "$TREE" apply --whitespace=nowarn "$PATCH"
    echo ">> applied $(basename "$PATCH")"
  else
    echo ">> the tree has local changes: patch NOT applied (regen records them; a fresh fetch resets)"
  fi
}

case "${1:-status}" in
  fetch)
    [ -e "$TREE" ] && { echo "!! $TREE exists; move it away first" >&2; exit 1; }
    mkdir -p "$(dirname "$TREE")"
    git -c advice.detachedHead=false clone --depth 1 --branch "$SDL_TAG" "$SDL_URL" "$TREE"
    got=$(git -C "$TREE" rev-parse HEAD)
    [ "$got" = "$SDL_COMMIT" ] || { echo "!! $SDL_TAG is $got, expected $SDL_COMMIT" >&2; exit 1; }
    apply
    ;;
  apply)
    apply
    ;;
  regen)
    mapfile -t new < <(git -C "$TREE" ls-files --others --exclude-standard)
    [ "${#new[@]}" -gt 0 ] && git -C "$TREE" add -N -- "${new[@]}"
    mkdir -p "$(dirname "$PATCH")"
    git -C "$TREE" diff HEAD > "$PATCH"
    echo ">> wrote $PATCH: $(wc -l < "$PATCH") lines, $(grep -c '^new file mode' "$PATCH" || true) new files"
    ;;
  status)
    echo "$(git -C "$TREE" describe --tags) $(git -C "$TREE" rev-parse --short HEAD) (pinned $SDL_TAG ${SDL_COMMIT:0:8})"
    git -C "$TREE" status --short | head -40
    ;;
  *)
    sed -n '2,16p' "$0"; exit 2
    ;;
esac
