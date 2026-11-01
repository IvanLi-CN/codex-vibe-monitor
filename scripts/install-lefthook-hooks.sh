#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"

custom_hooks_path="$(git -C "$repo_root" config --get core.hooksPath || true)"
if [ -n "$custom_hooks_path" ]; then
  printf '[worktree-bootstrap] core.hooksPath is set to %s; leaving hooks untouched\n' "$custom_hooks_path" >&2
  exit 0
fi

if [ "${CODEX_HOOKS_INSTALL_LOCK_HELD:-}" != '1' ]; then
  common_git_dir="$(git -C "$repo_root" rev-parse --path-format=absolute --git-common-dir)"
  install_lock_path="$common_git_dir/hooks-install.flock"
  hooks_directory="$(git -C "$repo_root" rev-parse --path-format=absolute --git-path hooks)"
  if [ -L "$hooks_directory" ] || [ ! -d "$hooks_directory" ]; then
    hooks_directory=''
  fi
  if perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -e '
    my ($lock_path, $script, $hooks_directory, @args) = @ARGV;
    my $lock_separator = rindex($lock_path, "/");
    die "unable to determine hooks install lock directory\n" if $lock_separator < 1;
    my $lock_directory = substr($lock_path, 0, $lock_separator);
    my $lock_name = substr($lock_path, $lock_separator + 1);
    my $caller_directory = abs_path(".")
      or die "unable to resolve current directory: $!\n";
    my $expected_directory = abs_path($lock_directory)
      or die "unable to resolve hooks install lock directory: $!\n";
    sysopen my $directory, $lock_directory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
      or die "unable to open hooks install lock directory: $!\n";
    flock($directory, LOCK_EX) or die "unable to acquire hooks install lock directory: $!\n";
    chdir $directory or die "unable to enter hooks install lock directory: $!\n";
    my $opened_directory = abs_path(".")
      or die "unable to resolve opened hooks install lock directory: $!\n";
    die "hooks install lock directory changed while opening\n"
      unless $opened_directory eq $expected_directory;
    die "hooks install lock must not be a symlink\n" if -l $lock_name;
    die "hooks install lock must be a regular file\n" if -e $lock_name && !-f $lock_name;
    sysopen my $lock, $lock_name, O_WRONLY | O_CREAT | O_APPEND | O_NOFOLLOW | O_NONBLOCK, 0600
      or die "unable to open hooks install lock: $!\n";
    sub verify_lock_identity {
      my ($handle, $name) = @_;
      my @handle_stat = stat($handle)
        or die "hooks install lock must be a regular file\n";
      my @path_stat = lstat($name)
        or die "hooks install lock must be a regular file\n";
      die "hooks install lock must not be a symlink\n"
        if (($path_stat[2] & S_IFMT) == S_IFLNK);
      die "hooks install lock must be a regular file\n"
        unless (($path_stat[2] & S_IFMT) == S_IFREG);
      die "hooks install lock path changed while opening\n"
        unless "$handle_stat[0]:$handle_stat[1]" eq "$path_stat[0]:$path_stat[1]";
    }
    verify_lock_identity($lock, $lock_name);
    flock($lock, LOCK_EX) or die "unable to acquire hooks install lock: $!\n";
    verify_lock_identity($lock, $lock_name);
    my $hooks;
    if (length $hooks_directory) {
      my $expected_hooks_directory = abs_path($hooks_directory)
        or die "unable to resolve hooks directory: $!\n";
      sysopen $hooks, $hooks_directory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
        or die "unable to open hooks directory: $!\n";
      my $opened_hooks_directory = abs_path($hooks_directory)
        or die "unable to resolve opened hooks directory: $!\n";
      die "hooks directory changed while opening\n"
        unless $opened_hooks_directory eq $expected_hooks_directory;
      flock($hooks, LOCK_EX) or die "unable to acquire hooks directory lock: $!\n";
    }
    chdir $caller_directory or die "unable to restore current directory: $!\n";
    $ENV{CODEX_HOOKS_INSTALL_LOCK_HELD} = 1;
    my $status = system { $script } $script, @args;
    exit 1 if $status == -1 || $status & 127;
    exit $status >> 8;
  ' "$install_lock_path" "$script_dir/install-lefthook-hooks.sh" "$hooks_directory" "$@"; then
    exit 0
  else
    install_lock_status=$?
    exit "$install_lock_status"
  fi
fi

lefthook_path=''
if ! command -v realpath >/dev/null 2>&1; then
  printf '[worktree-bootstrap] global lefthook is required; realpath is unavailable\n' >&2
  exit 1
fi

is_repo_local_path() {
  resolved_path="$1"
  case "$resolved_path" in
    "$repo_root"|"$repo_root"/*)
      return 0
      ;;
  esac

  while IFS= read -r worktree_root; do
    case "$resolved_path" in
      "$worktree_root"|"$worktree_root"/*)
        return 0
        ;;
    esac
  done < <(git -C "$repo_root" worktree list --porcelain 2>/dev/null | awk '/^worktree / {print substr($0, 10)}')

  return 1
}

old_ifs="$IFS"
IFS=:
for path_entry in $PATH; do
  [ -n "$path_entry" ] || path_entry='.'
  path_entry="$(cd "$path_entry" 2>/dev/null && pwd -P || true)"
  [ -n "$path_entry" ] || continue
  candidate="$path_entry/lefthook"
  case "$candidate" in
    "$repo_root"/node_modules/.bin/*|"$repo_root"/*/node_modules/.bin/*)
      continue
      ;;
  esac
  if [ -x "$candidate" ]; then
    resolved_candidate="$(realpath "$candidate" 2>/dev/null || true)"
    [ -n "$resolved_candidate" ] || continue
    if is_repo_local_path "$resolved_candidate"; then
      continue
    fi
    lefthook_path="$candidate"
    break
  fi
done
IFS="$old_ifs"

if [ -z "$lefthook_path" ]; then
  printf '[worktree-bootstrap] global lefthook is required; repo-local binary is not sufficient\n' >&2
  exit 1
fi

lefthook_version="$($lefthook_path version 2>/dev/null || true)"
lefthook_triplet="$(printf '%s\n' "$lefthook_version" | sed -nE 's/^[^0-9]*([0-9]+)\.([0-9]+)\.([0-9]+).*/\1 \2 \3/p' | head -n 1)"
set -- $lefthook_triplet
lefthook_major="${1:-}"
lefthook_minor="${2:-}"
lefthook_patch="${3:-}"
if [ -z "$lefthook_major" ] \
  || [ "$lefthook_major" -lt 2 ] \
  || { [ "$lefthook_major" -eq 2 ] && [ "$lefthook_minor" -lt 2 ]; } \
  || { [ "$lefthook_major" -eq 2 ] && [ "$lefthook_minor" -eq 2 ] && [ "$lefthook_patch" -lt 1 ]; }; then
  printf '[worktree-bootstrap] Lefthook 2.2.1 or newer is required for safe staged-file restoration; found %s\n' "${lefthook_version:-unknown}" >&2
  exit 1
fi

hooks_dir="$(git -C "$repo_root" rev-parse --git-path hooks)"
case "$hooks_dir" in
  /*) ;;
  *) hooks_dir="$repo_root/$hooks_dir" ;;
esac
hooks_parent="$(dirname "$hooks_dir")"
resolved_hooks_parent="$(realpath "$hooks_parent" 2>/dev/null || true)"
if [ -z "$resolved_hooks_parent" ] || [ "$resolved_hooks_parent" != "$hooks_parent" ]; then
  printf '[worktree-bootstrap] hooks directory has a symlinked parent; leaving hooks untouched: %s\n' "$hooks_dir" >&2
  exit 0
fi
if [ -L "$hooks_dir" ]; then
  printf '[worktree-bootstrap] hooks directory is a symlink; leaving hooks untouched: %s\n' "$hooks_dir" >&2
  exit 0
fi

config_declares_prepare_commit_msg() {
  for config_file in "$repo_root"/lefthook*.yml "$repo_root"/lefthook*.yaml; do
    [ -f "$config_file" ] || continue
    if grep -Eq '^[[:space:]]*prepare-commit-msg:' "$config_file"; then
      return 0
    fi
  done
  return 1
}

customize_pre_commit_template() {
  local template_hook_path="$1"
  local original_line='call_lefthook run "pre-commit" "$@"'

  [ "$(grep -Fxc "$original_line" "$template_hook_path" || true)" = '1' ] || return 1
  perl -0pi -e 's{call_lefthook run "pre-commit" "\$@"}{bash scripts/run-pre-commit-hook.sh "\$@"}' "$template_hook_path"
  grep -Fq 'bash scripts/run-pre-commit-hook.sh' "$template_hook_path"
}

customize_legacy_pre_commit_template() {
  local template_hook_path="$1"
  local original_line='call_lefthook run "pre-commit" "$@"'

  [ "$(grep -Fxc "$original_line" "$template_hook_path" || true)" = '1' ] || return 1
  perl -0pi -e 's{call_lefthook run "pre-commit" "\$@"}{if ! bash scripts/check-staged-formatter-safety.sh\nthen\n  exit 1\nfi\n\ncall_lefthook run "pre-commit" "\$@"}' "$template_hook_path"
  grep -Fq 'bash scripts/check-staged-formatter-safety.sh' "$template_hook_path"
}

cleanup_legacy_prepare_commit_msg() {
  hook_path="$hooks_dir/prepare-commit-msg"
  [ -e "$hook_path" ] || [ -L "$hook_path" ] || return 0

  if [ -L "$hook_path" ]; then
    printf '[worktree-bootstrap] prepare-commit-msg is a symlink; leaving it untouched\n' >&2
    return 0
  fi
  if [ ! -f "$hook_path" ]; then
    printf '[worktree-bootstrap] prepare-commit-msg is not a regular file; leaving it untouched\n' >&2
    return 0
  fi
  if config_declares_prepare_commit_msg; then
    printf '[worktree-bootstrap] prepare-commit-msg is configured; leaving it untouched\n' >&2
    return 0
  fi

  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")"
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! printf '\nprepare-commit-msg:\n  commands:\n    legacy-wrapper:\n      run: "true"\n' >> "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install prepare-commit-msg >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    printf '[worktree-bootstrap] could not verify prepare-commit-msg ownership; leaving it untouched\n' >&2
    return 0
  fi

  template_path="$template_dir/.git/hooks/prepare-commit-msg"
  hook_identity=''
  if hook_identity="$(stat -c '%d:%i' "$hook_path" 2>/dev/null)"; then
    :
  else
    hook_identity="$(stat -f '%d:%i' "$hook_path" 2>/dev/null || true)"
  fi
  current_hook_identity=''
  if current_hook_identity="$(stat -c '%d:%i' "$hook_path" 2>/dev/null)"; then
    :
  else
    current_hook_identity="$(stat -f '%d:%i' "$hook_path" 2>/dev/null || true)"
  fi
  if [ -f "$template_path" ] \
    && [ -n "$hook_identity" ] \
    && [ "$current_hook_identity" = "$hook_identity" ] \
    && cmp -s "$hook_path" "$template_path"; then
    final_hook_identity=''
    if final_hook_identity="$(stat -c '%d:%i' "$hook_path" 2>/dev/null)"; then
      :
    else
      final_hook_identity="$(stat -f '%d:%i' "$hook_path" 2>/dev/null || true)"
    fi
    if [ "$final_hook_identity" = "$hook_identity" ] && cmp -s "$hook_path" "$template_path"; then
      rm -f "$hook_path"
      printf '[worktree-bootstrap] removed obsolete Lefthook prepare-commit-msg wrapper\n'
    else
      printf '[worktree-bootstrap] prepare-commit-msg changed during migration; leaving it untouched\n' >&2
    fi
  else
    printf '[worktree-bootstrap] prepare-commit-msg is unmanaged; leaving it untouched\n' >&2
  fi
  rm -rf "$template_dir"
}

cleanup_legacy_prepare_commit_msg

hook_identity() {
  local hook_path="$1"
  local identity=''

  [ -f "$hook_path" ] || return 1
  [ ! -L "$hook_path" ] || return 1
  if identity="$(stat -c '%d:%i' "$hook_path" 2>/dev/null)"; then
    :
  else
    identity="$(stat -f '%d:%i' "$hook_path" 2>/dev/null || true)"
  fi
  [ -n "$identity" ] || return 1
  printf '%s\n' "$identity"
}

hook_content_hash() {
  local hook_path="$1"

  [ -f "$hook_path" ] || return 1
  [ ! -L "$hook_path" ] || return 1
  git -C "$repo_root" hash-object -- "$hook_path"
}

hook_matches_snapshot() {
  local hook_path="$1"
  local expected_identity="$2"
  local expected_hash="$3"
  local current_identity current_hash

  current_identity="$(hook_identity "$hook_path" 2>/dev/null || true)"
  current_hash="$(hook_content_hash "$hook_path" 2>/dev/null || true)"
  [ "$current_identity" = "$expected_identity" ] \
    && [ "$current_hash" = "$expected_hash" ]
}

is_installable_hook() {
  local hook_name="$1"
  local hook_path="$2"

  is_managed_hook "$hook_name" "$hook_path" \
    || { [ "$hook_name" = 'pre-commit' ] && is_marked_standard_pre_commit "$hook_path"; }
}

install_hooks=()
is_marked_standard_pre_commit() {
  local existing_hook_path="$1"
  local template_dir template_path

  [ -L "$existing_hook_path" ] && return 1
  [ -f "$existing_hook_path" ] || return 1
  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")" || return 1
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install pre-commit >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    return 1
  fi

  template_path="$template_dir/.git/hooks/pre-commit"
  printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$template_path"
  if [ -f "$template_path" ] && cmp -s "$existing_hook_path" "$template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  rm -rf "$template_dir"
  return 1
}

is_managed_hook() {
  hook_name="$1"
  hook_path="$2"
  if [ ! -e "$hook_path" ] && [ ! -L "$hook_path" ]; then
    return 0
  fi
  if [ -L "$hook_path" ]; then
    return 1
  fi
  if [ ! -f "$hook_path" ]; then
    return 1
  fi

  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")" || return 1
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install "$hook_name" >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    return 1
  fi

  template_path="$template_dir/.git/hooks/$hook_name"
  printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$template_path"
  if [ -f "$template_path" ] && cmp -s "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    [ "$hook_name" = 'pre-commit' ] && return 1
    return 0
  fi
  if [ "$hook_name" = 'pre-commit' ] \
    && legacy_template_path="$template_dir/pre-commit.legacy" \
    && cp "$template_path" "$legacy_template_path" \
    && customize_legacy_pre_commit_template "$legacy_template_path" \
    && [ -f "$legacy_template_path" ] \
    && cmp -s "$hook_path" "$legacy_template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  if [ "$hook_name" = 'pre-commit' ] \
    && customize_pre_commit_template "$template_path" \
    && [ -f "$template_path" ] \
    && cmp -s "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  rm -rf "$template_dir"
  return 1
}

for hook_name in pre-commit commit-msg post-checkout; do
  hook_path="$hooks_dir/$hook_name"
  if is_installable_hook "$hook_name" "$hook_path"; then
    install_hooks+=("$hook_name")
  else
    printf '[worktree-bootstrap] %s already exists and is unmanaged; leaving it untouched\n' "$hook_name" >&2
  fi
done

if [ "${#install_hooks[@]}" -gt 0 ]; then
  for hook_name in "${install_hooks[@]}"; do
    hook_path="$hooks_dir/$hook_name"
    if ! is_installable_hook "$hook_name" "$hook_path"; then
      printf '[worktree-bootstrap] %s changed during migration; leaving hooks untouched\n' "$hook_name" >&2
      exit 1
    fi
  done
  (
    cd "$repo_root"
    "$lefthook_path" install "${install_hooks[@]}"
    for hook_name in "${install_hooks[@]}"; do
      hook_path="$hooks_dir/$hook_name"
      installed_identity="$(hook_identity "$hook_path" 2>/dev/null || true)"
      installed_hash="$(hook_content_hash "$hook_path" 2>/dev/null || true)"
      if [ -z "$installed_identity" ] || [ -z "$installed_hash" ]; then
        printf '[worktree-bootstrap] %s was not installed as a regular hook\n' "$hook_name" >&2
        exit 1
      fi
      if ! hook_matches_snapshot "$hook_path" "$installed_identity" "$installed_hash"; then
        printf '[worktree-bootstrap] %s changed after Lefthook install; leaving it untouched\n' "$hook_name" >&2
        exit 1
      fi
      if [ "$hook_name" = 'pre-commit' ] \
        && ! customize_pre_commit_template "$hook_path"; then
        printf '[worktree-bootstrap] could not install pre-commit safety check\n' >&2
        exit 1
      fi
      marker_identity="$(hook_identity "$hook_path" 2>/dev/null || true)"
      marker_hash="$(hook_content_hash "$hook_path" 2>/dev/null || true)"
      if [ -z "$marker_identity" ] || [ -z "$marker_hash" ]; then
        printf '[worktree-bootstrap] %s changed before marker installation; leaving it untouched\n' "$hook_name" >&2
        exit 1
      fi
      if ! hook_matches_snapshot "$hook_path" "$marker_identity" "$marker_hash"; then
        printf '[worktree-bootstrap] %s changed before marker installation; leaving it untouched\n' "$hook_name" >&2
        exit 1
      fi
      if [ "$hook_name" != 'pre-commit' ] \
        && { [ "$marker_identity" != "$installed_identity" ] || [ "$marker_hash" != "$installed_hash" ]; }; then
        printf '[worktree-bootstrap] %s changed before marker installation; leaving it untouched\n' "$hook_name" >&2
        exit 1
      fi
      printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$hook_path"
    done
  )
fi
