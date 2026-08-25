#!/usr/bin/env bash
# Codex-only, headless expert dispatcher for the Clannon workspace.
#
#   ./scripts/crew.sh run <role> --brief <file> [--dir <path>]...
#   ./scripts/crew.sh status
#
# Workers share the Git worktree but receive one explicit path set. They never
# commit; the coordinator reviews and integrates their changes.

set -uo pipefail

CREW_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CREW_RUNS="$CREW_ROOT/.agents/runs"
CREW_ROLES="runtime server experience verification architecture review"

die() {
  echo "crew: $*" >&2
  exit 2
}

role_dir() {
  case "$1" in
    runtime)      echo "$CREW_ROOT/crates/runtime" ;;
    server)       echo "$CREW_ROOT/crates/server" ;;
    experience)   echo "$CREW_ROOT/static" ;;
    verification) echo "$CREW_ROOT/tests" ;;
    architecture|review) echo "$CREW_ROOT" ;;
    *) return 1 ;;
  esac
}

valid_role() {
  role_dir "$1" >/dev/null 2>&1
}

resolve_owned_path() {
  local candidate="$1" resolved parent
  case "$candidate" in
    /*) ;;
    *) candidate="$CREW_ROOT/$candidate" ;;
  esac

  if [ -e "$candidate" ]; then
    resolved="$(readlink -f "$candidate")"
  else
    parent="$(readlink -f "$(dirname "$candidate")" 2>/dev/null || true)"
    [ -n "$parent" ] && [ -d "$parent" ] \
      || die "owned path and its parent do not exist: $candidate"
    resolved="$parent/$(basename "$candidate")"
  fi

  case "$resolved" in
    "$CREW_ROOT"|"$CREW_ROOT"/*) ;;
    *) die "owned path escapes repository: $candidate" ;;
  esac
  case "$resolved" in
    "$CREW_ROOT/.git"|"$CREW_ROOT/.git"/*|"$CREW_ROOT/target"|"$CREW_ROOT/target"/*)
      die "owned path is never dispatchable: $resolved" ;;
  esac
  echo "$resolved"
}

show_status() {
  local role lock state pid
  printf 'Clannon workers — %s\n\n' "$(date '+%F %T %Z')"
  printf '  %-14s %s\n' ROLE STATE
  for role in $CREW_ROLES; do
    lock="$CREW_RUNS/$role.lock"
    state="idle"
    if [ -f "$lock" ]; then
      pid="$(sed -n '1p' "$lock" 2>/dev/null)"
      if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        state="running (pid $pid)"
      else
        state="stale lock"
      fi
    fi
    printf '  %-14s %s\n' "$role" "$state"
  done
}

run_worker() {
  local role="" brief="" next="" arg default_dir cwd lock stamp output log
  local started ended rc=0 before_head after_head
  local before_status after_status violation=""
  local -a requested=() owned=()

  for arg in "$@"; do
    case "$next" in
      brief) brief="$arg"; next=""; continue ;;
      dir) requested+=("$arg"); next=""; continue ;;
    esac
    case "$arg" in
      --brief) next=brief ;;
      --dir) next=dir ;;
      --*) die "unknown flag: $arg" ;;
      *) role="$arg" ;;
    esac
  done

  [ -z "$next" ] || die "missing value for --$next"
  [ -n "$role" ] && [ -n "$brief" ] \
    || die "usage: crew.sh run <role> --brief <file> [--dir <path>]..."
  valid_role "$role" || die "unknown role: $role (roles: $CREW_ROLES)"
  command -v codex >/dev/null 2>&1 || die "Codex CLI not found"

  brief="$(readlink -f "$brief" 2>/dev/null || true)"
  [ -n "$brief" ] && [ -f "$brief" ] || die "brief not found"
  case "$brief" in
    "$CREW_ROOT"/*) ;;
    *) die "brief must live inside this repository" ;;
  esac

  default_dir="$(role_dir "$role")"
  if [ "${#requested[@]}" -eq 0 ]; then
    case "$role" in
      architecture|review)
        die "$role workers require at least one --dir path" ;;
      *) requested+=("$default_dir") ;;
    esac
  fi
  for arg in "${requested[@]}"; do
    owned+=("$(resolve_owned_path "$arg")")
  done

  cwd="${owned[0]}"
  [ -d "$cwd" ] || cwd="$(dirname "$cwd")"
  mkdir -p "$CREW_RUNS" "$CREW_ROOT/reports/$role" \
    "$CREW_ROOT/proposals/to-coordinator" "$CREW_ROOT/comms/$(date +%F)"
  lock="$CREW_RUNS/$role.lock"

  if [ -f "$lock" ]; then
    local existing_pid
    existing_pid="$(sed -n '1p' "$lock" 2>/dev/null)"
    if [ -n "$existing_pid" ] && kill -0 "$existing_pid" 2>/dev/null; then
      die "$role worker already running (pid $existing_pid)"
    fi
    rm -f "$lock"
  fi

  if [ -n "$(git -C "$CREW_ROOT" status --porcelain -- "${owned[@]}")" ]; then
    echo "crew: owned paths contain uncommitted changes:" >&2
    git -C "$CREW_ROOT" status --short -- "${owned[@]}" >&2
    die "commit or resolve them before dispatch"
  fi

  stamp="$(date +%Y%m%d-%H%M%S)"
  output="$CREW_RUNS/$stamp-$role.out"
  log="$CREW_RUNS/$stamp-$role.log"
  before_status="$CREW_RUNS/$stamp-$role.before-status"
  after_status="$CREW_RUNS/$stamp-$role.after-status"
  before_head="$(git -C "$CREW_ROOT" rev-parse HEAD)"
  git -C "$CREW_ROOT" status --porcelain | sort > "$before_status"

  local mandate full_brief
  mandate="$(cat <<EOF

---
DISPATCH MANDATE (automatically added by scripts/crew.sh):
- You are the '$role' expert. The primary Codex session is product coordinator,
  reviewer, integrator, and the only agent allowed to commit.
- Read $CREW_ROOT/AGENTS.md and $CREW_ROOT/PROJECT.md before acting.
- You own exactly these paths for this task:
$(printf '  - %s\n' "${owned[@]}")
- Do not edit any other source path. If the brief requires one, stop and report
  the missing scope instead of crossing the boundary.
- Do not commit, amend, reset, stash, checkout, push, or mutate remotes.
- Deep evidence may be written to $CREW_ROOT/reports/$role/ using the format in
  reports/README.md. A decision requiring coordinator ruling may be written to
  $CREW_ROOT/proposals/to-coordinator/ using proposals/README.md. Those are the
  only write exceptions to the owned path list.
- Verify the premise before changing code. If it is false, make no code change.
- Run task-relevant checks and report commands with observed results.
- Final response must be self-contained: findings, changed paths, verification,
  and anything deliberately left undone.
EOF
)"
  full_brief="$(cat "$brief")$mandate"

  printf '%s\n' "$$" "${owned[@]}" > "$lock"
  trap "rm -f '$lock'" EXIT INT TERM
  started="$(date +%s)"
  echo "crew: dispatching $role expert [codex]"
  echo "crew: brief  $brief"
  echo "crew: cwd    $cwd"
  echo "crew: output $output"

  codex exec "$full_brief" -C "$cwd" --skip-git-repo-check \
    --sandbox danger-full-access -o "$output" </dev/null >"$log" 2>&1 || rc=$?

  ended="$(date +%s)"
  rm -f "$lock"
  trap - EXIT INT TERM
  after_head="$(git -C "$CREW_ROOT" rev-parse HEAD)"
  if [ "$before_head" != "$after_head" ]; then
    echo "crew: ERROR — worker changed HEAD; coordinator review required" >&2
    rc=4
  fi

  # Detect newly dirty paths outside the granted source scope. Existing dirty
  # paths are tolerated so independent lanes can run in parallel, but a worker
  # cannot quietly create a new cross-lane edit.
  git -C "$CREW_ROOT" status --porcelain | sort > "$after_status"
  while IFS= read -r arg; do
    [ -n "$arg" ] || continue
    local changed_path="${arg:3}" allowed=no owned_path relative_owned
    for owned_path in "${owned[@]}"; do
      relative_owned="${owned_path#"$CREW_ROOT"/}"
      case "$changed_path" in
        "$relative_owned"|"$relative_owned"/*) allowed=yes; break ;;
      esac
    done
    case "$changed_path" in
      "reports/$role/"*|proposals/to-coordinator/*) allowed=yes ;;
    esac
    if [ "$allowed" = no ]; then
      violation="${violation}${violation:+, }$changed_path"
    fi
  done < <(comm -13 "$before_status" "$after_status")
  if [ -n "$violation" ]; then
    echo "crew: ERROR — worker dirtied paths outside its scope: $violation" >&2
    rc=5
  fi

  local comms="$CREW_ROOT/comms/$(date +%F)/coordinator.md"
  if ! grep -q '^## dispatched experts' "$comms" 2>/dev/null; then
    printf '\n## dispatched experts\n' >> "$comms"
  fi
  printf -- '- `%s` **%s** via **codex** — %s — exit %s, %ss — `%s`\n' \
    "$(date +%H:%M)" "$role" "$(basename "$brief")" "$rc" \
    "$((ended-started))" ".agents/runs/$(basename "$output")" >> "$comms"

  echo "crew: worker finished — exit $rc, $((ended-started))s"
  echo "crew: --- final response ---"
  cat "$output" 2>/dev/null
  return "$rc"
}

case "${1:-}" in
  run) shift; run_worker "$@" ;;
  status) show_status ;;
  -h|--help|"") sed -n '2,8p' "$0" ;;
  *) die "unknown command: $1 (use run or status)" ;;
esac
