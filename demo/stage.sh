#!/usr/bin/env bash
# A staged home for the README pictures: a canon that was just moved from
# ~/dotfiles/development to ~/dotfiles/canon, a Claude whose CLAUDE.md still
# imports the old path, a Codex with nothing wired yet, a few projects importing
# house files from the old folder, and skills in both places. Nothing here
# touches your real home: every path, XDG variables included, is redirected
# into ./home.
#
#   ./stage.sh up     build it, not set up yet (the setup wizard opens)
#   ./stage.sh ready  build it and set it up, drift left in place for the stills
#   ./stage.sh run    launch canon against it (this is what you screenshot)
#   ./stage.sh shell  a shell where `canon` is this build, for the CLI shots
#   ./stage.sh down   delete it
#
# The shell it opens wears the same invented `user@host` prompt as every other
# crate's rig. Every name in it is invented.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

STAGE="$HERE/home"
BIN="$STAGE/.bin"
CANON="$HERE/../target/release/canon"
TEMPLATES="$HERE/../templates"

# Written by `up`, required by `down`. See the guard further down.
MARKER=".canon-demo-stage"

# The complete environment anything staged runs in. Used with `env -i`, so this
# is not "the real environment plus overrides": it is everything there is.
env_for_stage() {
  echo "HOME=$STAGE" \
       "XDG_CONFIG_HOME=$STAGE/.config" \
       "XDG_DATA_HOME=$STAGE/.local/share" \
       "XDG_STATE_HOME=$STAGE/.local/state" \
       "XDG_CACHE_HOME=$STAGE/.cache" \
       "EDITOR=true" \
       "PATH=$BIN:/usr/local/bin:/usr/bin:/bin" \
       "TERM=${TERM:-xterm-256color}" \
       "COLORTERM=truecolor" \
       "LANG=C.UTF-8"
}

skill() {
  mkdir -p "$1"
  printf -- '---\nname: %s\ndescription: %s\n---\n\n%s\n' "$(basename "$1")" "$2" "$2" > "$1/SKILL.md"
}

write_canon() {
  local c="$STAGE/dotfiles/canon"
  mkdir -p "$c/house"
  cp "$TEMPLATES/rules.yaml" "$c/rules.yaml"
  cp "$TEMPLATES/rules.schema.json" "$c/rules.schema.json"
  printf '# HOUSE-RUST.md\n\nHow the Rust crates are checked and shipped.\n' > "$c/house/HOUSE-RUST.md"
  printf '# HOUSE-TUI.md\n\nHow the terminal interfaces are built.\n' > "$c/house/HOUSE-TUI.md"
  skill "$c/skills/code-review" "Review the current changes against the rules"
  skill "$c/skills/release" "Walk a crate through its release steps"
}

write_agents() {
  # Claude still imports the rules from where the canon used to be.
  mkdir -p "$STAGE/.claude/skills/synced"
  printf '@~/dotfiles/development/rules.yaml\n\nPrefer short answers.\n' > "$STAGE/.claude/CLAUDE.md"
  skill "$STAGE/.claude/skills/meeting-notes" "Turn a transcript into notes"
  # Codex, pi and opencode are installed and know nothing yet.
  mkdir -p "$STAGE/.codex" "$STAGE/.pi/agent" "$STAGE/.config/opencode"
}

write_projects() {
  local d="$STAGE/dev"
  mkdir -p "$d/crates/api" "$d/crates/cli" "$d/web/site"
  printf '@~/dotfiles/development/HOUSE-RUST.md\n@~/dotfiles/development/HOUSE-TUI.md\n@AGENTS.md\n' > "$d/crates/api/CLAUDE.md"
  printf '# api\n' > "$d/crates/api/AGENTS.md"
  printf '@~/dotfiles/development/HOUSE-RUST.md\n@AGENTS.md\n' > "$d/crates/cli/CLAUDE.md"
  printf '# cli\n' > "$d/crates/cli/AGENTS.md"
  printf '# site\n' > "$d/web/site/AGENTS.md"
  local r
  for r in "$d/crates/api" "$d/crates/cli" "$d/web/site"; do
    env -i $(env_for_stage) git -C "$r" init -q
    printf 'CLAUDE.md\n' > "$r/.gitignore"
  done
}

# Set up the way the wizard would, by running setup itself: a hand-written
# copy of canonize.toml would show frames of a config the tool never writes.
write_setup() {
  (cd "$STAGE" && env -i $(env_for_stage) "$CANON" setup > /dev/null)
}

up() {
  down_quiet
  mkdir -p "$STAGE"
  # Stamp it before anything else, so a later `down` can prove this tree is ours.
  : > "$STAGE/$MARKER"
  write_canon
  write_agents
  write_projects
  [ -z "${READY:-}" ] || write_setup
  # A shell rc that sources something out of the real home would error on it
  # here; an empty stand-in keeps the staged shell quiet.
  mkdir -p "$STAGE/.cargo" && : > "$STAGE/.cargo/env"
  echo "staged in $STAGE"
}

# ---------------------------------------------------------------------------
# the teardown guard: identical in every crate's rig
# ---------------------------------------------------------------------------
# A rig is a convenience script with a recursive delete in it, run half
# attentively while thinking about something else, against a path some scenario
# may have mounted a remote filesystem onto. Both halves of that have already
# happened in this workflow: a stage path that pointed somewhere real and was
# deleted because the script trusted its own variable, and an sshfs mount inside
# a staged home torn down with `rm -rf`, which walked through the mountpoint and
# deleted the dotfiles on the machine at the far end. So the delete is proved
# rather than trusted.
refuse() { echo "REFUSING to delete $STAGE: $1" >&2; exit 1; }

assert_safe_to_delete() {
  case "$STAGE" in
    /*) ;;
    *) refuse "the stage path must be absolute" ;;
  esac
  # Resolve symlinks first: a link pointing the stage at something real must not
  # let a delete through on the strength of a harmless-looking path.
  local real
  real="$(cd "$STAGE" && pwd -P)" || refuse "cannot resolve the path"
  case "$real" in
    / | /home | /root | /usr | /etc | /var | /opt | /srv | /boot | /tmp)
      refuse "that is a system directory" ;;
  esac
  [ "$real" = "$HOME" ] && refuse "that is your home directory"
  case "$HOME/" in
    "$real"/*) refuse "your home directory is inside it" ;;
  esac
  # The real gate: only ever delete a tree this script built and stamped.
  [ -f "$real/$MARKER" ] || refuse "no \`$MARKER\` in it, so this script did not build it"
  # Unmount anything under it, longest path first, then check again: a recursive
  # delete walks straight through a mountpoint and removes the far side.
  local mp
  while read -r mp; do
    [ -n "$mp" ] || continue
    echo "unmounting $mp"
    fusermount -u "$mp" 2> /dev/null || umount "$mp" 2> /dev/null || true
  done < <(awk -v s="$real/" '$2 ~ "^"s {print length($2), $2}' /proc/mounts |
             sort -rn | cut -d' ' -f2-)
  if awk -v s="$real/" '$2 ~ "^"s {found=1} END {exit !found}' /proc/mounts; then
    refuse "something is still mounted under it; unmount it by hand and rerun"
  fi
}

down_quiet() {
  [ -d "$STAGE" ] || return 0
  assert_safe_to_delete
  # --one-file-system as a second net, in case the mount check was wrong.
  rm -rf --one-file-system "$STAGE"
}

# ---------------------------------------------------------------------------
# the shell in frame: identical in every crate's rig
# ---------------------------------------------------------------------------
# The prompt is invented, and deliberately not the renderer's own. Sourcing a
# real ~/.bashrc paints a different picture on every machine that regenerates
# the assets, which defeats the point of keeping the rig in the repo: these
# images are a build output, and a build output that depends on whose machine
# ran it is not reproducible. A username is not a leak, but `user@host` is the
# same for everyone, and it is the same string in every rig so the frames
# match. Every tape sets the same theme and font for the same reason.
write_demorc() {
  cat > "$STAGE/.demorc" <<'EOF'
PS1='\[\e[38;5;114m\]user@host\[\e[0m\]:\[\e[38;5;110m\]\w\[\e[0m\]\$ '
unset PROMPT_COMMAND
HISTFILE=
clear
EOF
}

# A shell that finds this build as `canon`, so a CLI screenshot shows the
# command you actually type. It starts in the staged home.
# Both `run` and `shell` refuse without a stage: `shell` used to build half of
# one, which left `demo/home` without the marker and made every later `down`,
# `up` and `ready` refuse for good.
need_stage() {
  [ -f "$STAGE/$MARKER" ] || {
    echo "no stage in $STAGE: run ./stage.sh up (or ready) first" >&2
    exit 1
  }
}

open_shell() {
  need_stage
  mkdir -p "$BIN"
  ln -sf "$(cd "$(dirname "$CANON")" && pwd)/canon" "$BIN/canon"
  write_demorc
  (cd "$STAGE" && env -i $(env_for_stage) \
    bash --noprofile --rcfile "$STAGE/.demorc" -i)
}

case "${1:-up}" in
  up)    up ;;
  ready) READY=1 up ;;
  run)   need_stage; (cd "$STAGE" && env -i $(env_for_stage) "$CANON") ;;
  shell) open_shell ;;
  down)  down_quiet; echo "torn down" ;;
  *)     echo "usage: $0 [up|ready|run|shell|down]" >&2; exit 2 ;;
esac
