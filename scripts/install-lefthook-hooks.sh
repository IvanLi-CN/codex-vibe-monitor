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
      $ENV{CODEX_HOOKS_DIRECTORY_LOCK_HELD} = 1;
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

customize_post_checkout_template() {
  local template_hook_path="$1"
  local original_line='call_lefthook run "post-checkout" "$@"'

  [ "$(grep -Fxc "$original_line" "$template_hook_path" || true)" = '1' ] || return 1
  perl -0pi -e 's{call_lefthook run "post-checkout" "\$@"}{call_lefthook run "post-checkout" --no-auto-install "\$@"}' "$template_hook_path"
  grep -Fq 'call_lefthook run "post-checkout" --no-auto-install' "$template_hook_path"
}

hook_snapshot() {
  local hook_path="$1"

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -MIPC::Open2 -e '
    my ($repo_root, $hook_path) = @ARGV;
    my $separator = rindex($hook_path, "/");
    die "unable to determine hook parent directory\n" if $separator < 1;
    my $directory_path = substr($hook_path, 0, $separator);
    my $hook_name = substr($hook_path, $separator + 1);
    my $expected_directory = abs_path($directory_path)
      or die "unable to resolve hook parent directory: $!\n";
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
      or die "unable to open hook parent directory: $!\n";
    flock($directory, LOCK_SH) unless $ENV{CODEX_HOOKS_DIRECTORY_LOCK_HELD};
    chdir $directory or die "unable to enter hook parent directory: $!\n";
    my $opened_directory = abs_path(".")
      or die "unable to resolve opened hook parent directory: $!\n";
    die "hook parent directory changed while opening\n"
      unless $opened_directory eq $expected_directory;
    sysopen my $hook, $hook_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open hook: $!\n";
    binmode $hook;
    flock($hook, LOCK_SH) or die "unable to lock hook: $!\n";

    sub regular_identity {
      my ($handle, $name) = @_;
      my @handle_stat = stat($handle);
      my @path_stat = lstat($name);
      return undef unless @handle_stat && @path_stat;
      return undef unless (($handle_stat[2] & S_IFMT) == S_IFREG);
      return undef unless (($path_stat[2] & S_IFMT) == S_IFREG);
      my $handle_identity = "$handle_stat[0]:$handle_stat[1]";
      my $path_identity = "$path_stat[0]:$path_stat[1]";
      return undef unless $handle_identity eq $path_identity;
      return $handle_identity;
    }

    sub read_contents {
      my ($handle) = @_;
      seek($handle, 0, 0) or return undef;
      my $contents = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($handle, $chunk, 65536);
        return undef unless defined $count;
        last if $count == 0;
        $contents .= $chunk;
      }
      return $contents;
    }

    sub hash_contents {
      my ($contents) = @_;
      my ($read_handle, $write_handle);
      my $pid = eval {
        open2($read_handle, $write_handle, "git", "-C", $repo_root, "hash-object", "--stdin")
      };
      return undef unless defined $pid;
      binmode $read_handle;
      binmode $write_handle;
      my $offset = 0;
      my $length = length($contents);
      while ($offset < $length) {
        my $count = syswrite($write_handle, $contents, $length - $offset, $offset);
        return undef unless defined($count) && $count > 0;
        $offset += $count;
      }
      close $write_handle or return undef;
      my $hash = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($read_handle, $chunk, 128);
        return undef unless defined $count;
        last if $count == 0;
        $hash .= $chunk;
      }
      close $read_handle or return undef;
      waitpid($pid, 0);
      return undef unless $? == 0;
      $hash =~ s/[\r\n]+\z//;
      return $hash if $hash =~ /\A[0-9a-f]+\z/;
      return undef;
    }

    my $identity = regular_identity($hook, $hook_name)
      or die "hook must be a regular file\n";
    my $first_contents = read_contents($hook);
    die "unable to read hook\n" unless defined $first_contents;
    die "hook changed while reading\n" unless regular_identity($hook, $hook_name);
    my $second_contents = read_contents($hook);
    die "unable to read hook\n" unless defined $second_contents;
    die "hook changed while reading\n" unless $first_contents eq $second_contents;
    my $hash = hash_contents($second_contents)
      or die "unable to hash hook\n";
    die "hook changed while hashing\n" unless regular_identity($hook, $hook_name);
    my $final_contents = read_contents($hook);
    die "unable to read hook\n" unless defined $final_contents;
    die "hook changed while hashing\n" unless $final_contents eq $second_contents;
    print "$identity\t$hash\n";
  ' "$repo_root" "$hook_path"
}

hook_file_matches() {
  local hook_path="$1"
  local expected_path="$2"

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -e '
    my ($hook_path, $expected_path) = @ARGV;
    my $separator = rindex($hook_path, "/");
    die "unable to determine hook parent directory\n" if $separator < 1;
    my $directory_path = substr($hook_path, 0, $separator);
    my $hook_name = substr($hook_path, $separator + 1);
    my $expected_directory = abs_path($directory_path)
      or die "unable to resolve hook parent directory: $!\n";
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
      or die "unable to open hook parent directory: $!\n";
    flock($directory, LOCK_SH) unless $ENV{CODEX_HOOKS_DIRECTORY_LOCK_HELD};
    chdir $directory or die "unable to enter hook parent directory: $!\n";
    my $opened_directory = abs_path(".")
      or die "unable to resolve opened hook parent directory: $!\n";
    die "hook parent directory changed while opening\n"
      unless $opened_directory eq $expected_directory;
    sysopen my $hook, $hook_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open hook: $!\n";
    sysopen my $expected, $expected_path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open expected hook: $!\n";
    binmode $hook;
    binmode $expected;
    flock($hook, LOCK_SH) or die "unable to lock hook: $!\n";

    sub regular_identity {
      my ($handle, $name) = @_;
      my @handle_stat = stat($handle);
      my @path_stat = lstat($name);
      return undef unless @handle_stat && @path_stat;
      return undef unless (($handle_stat[2] & S_IFMT) == S_IFREG);
      return undef unless (($path_stat[2] & S_IFMT) == S_IFREG);
      return undef unless "$handle_stat[0]:$handle_stat[1]" eq "$path_stat[0]:$path_stat[1]";
      return 1;
    }

    sub read_contents {
      my ($handle) = @_;
      seek($handle, 0, 0) or return undef;
      my $contents = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($handle, $chunk, 65536);
        return undef unless defined $count;
        last if $count == 0;
        $contents .= $chunk;
      }
      return $contents;
    }

    regular_identity($hook, $hook_name) or die "hook must be a regular file\n";
    regular_identity($expected, $expected_path) or die "expected hook must be a regular file\n";
    my $hook_contents = read_contents($hook);
    my $expected_contents = read_contents($expected);
    die "unable to read hook\n" unless defined($hook_contents) && defined($expected_contents);
    regular_identity($hook, $hook_name) or die "hook changed while reading\n";
    exit($hook_contents eq $expected_contents ? 0 : 1);
  ' "$hook_path" "$expected_path"
}

safe_remove_legacy_hook() {
  local hook_path="$1"
  local template_path="$2"
  local expected_identity="$3"

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -e '
    my ($hook_path, $template_path, $expected_identity) = @ARGV;
    my $separator = rindex($hook_path, "/");
    die "unable to determine hook parent directory\n" if $separator < 1;
    my $directory_path = substr($hook_path, 0, $separator);
    my $hook_name = substr($hook_path, $separator + 1);
    my $expected_directory = abs_path($directory_path)
      or die "unable to resolve hook parent directory: $!\n";
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
      or die "unable to open hook parent directory: $!\n";
    flock($directory, LOCK_EX) unless $ENV{CODEX_HOOKS_DIRECTORY_LOCK_HELD};
    chdir $directory or die "unable to enter hook parent directory: $!\n";
    my $opened_directory = abs_path(".")
      or die "unable to resolve opened hook parent directory: $!\n";
    die "hook parent directory changed while opening\n"
      unless $opened_directory eq $expected_directory;
    sysopen my $hook, $hook_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open hook: $!\n";
    sysopen my $template, $template_path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open hook template: $!\n";
    binmode $hook;
    binmode $template;
    flock($hook, LOCK_EX) or die "unable to lock hook: $!\n";

    sub regular_identity {
      my ($handle, $name) = @_;
      my @handle_stat = stat($handle);
      my @path_stat = lstat($name);
      return undef unless @handle_stat && @path_stat;
      return undef unless (($handle_stat[2] & S_IFMT) == S_IFREG);
      return undef unless (($path_stat[2] & S_IFMT) == S_IFREG);
      return undef unless "$handle_stat[0]:$handle_stat[1]" eq "$path_stat[0]:$path_stat[1]";
      return "$handle_stat[0]:$handle_stat[1]";
    }

    sub read_contents {
      my ($handle) = @_;
      seek($handle, 0, 0) or return undef;
      my $contents = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($handle, $chunk, 65536);
        return undef unless defined $count;
        last if $count == 0;
        $contents .= $chunk;
      }
      return $contents;
    }

    my $identity = regular_identity($hook, $hook_name)
      or die "hook must be a regular file\n";
    die "hook identity changed\n" unless $identity eq $expected_identity;
    regular_identity($template, $template_path) or die "hook template must be a regular file\n";
    my $hook_contents = read_contents($hook);
    my $template_contents = read_contents($template);
    die "unable to read hook\n" unless defined($hook_contents) && defined($template_contents);
    die "hook contents changed\n" unless $hook_contents eq $template_contents;
    die "hook identity changed\n" unless (regular_identity($hook, $hook_name) || "") eq $expected_identity;
    my $final_contents = read_contents($hook);
    die "unable to read hook\n" unless defined $final_contents;
    die "hook contents changed\n" unless $final_contents eq $template_contents;
    die "hook identity changed\n" unless (regular_identity($hook, $hook_name) || "") eq $expected_identity;
    unlink($hook_name) or die "unable to remove hook: $!\n";
  ' "$hook_path" "$template_path" "$expected_identity"
}

safe_publish_hook() {
  local hook_path="$1"
  local source_path="$2"
  local expected_state="$3"
  local expected_identity="$4"
  local expected_hash="$5"

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -MIPC::Open2 -e '
    my ($repo_root, $hook_path, $source_path, $expected_state, $expected_identity, $expected_hash) = @ARGV;
    my $separator = rindex($hook_path, "/");
    die "unable to determine hook parent directory\n" if $separator < 1;
    my $directory_path = substr($hook_path, 0, $separator);
    my $hook_name = substr($hook_path, $separator + 1);
    my $expected_directory = abs_path($directory_path)
      or die "unable to resolve hook parent directory: $!\n";
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
      or die "unable to open hook parent directory: $!\n";
    flock($directory, LOCK_EX) unless $ENV{CODEX_HOOKS_DIRECTORY_LOCK_HELD};
    chdir $directory or die "unable to enter hook parent directory: $!\n";
    my $opened_directory = abs_path(".")
      or die "unable to resolve opened hook parent directory: $!\n";
    die "hook parent directory changed while opening\n"
      unless $opened_directory eq $expected_directory;
    sysopen my $source, $source_path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
      or die "unable to open generated hook: $!\n";
    binmode $source;

    sub regular_identity {
      my ($handle, $name) = @_;
      my @handle_stat = stat($handle);
      my @path_stat = lstat($name);
      return undef unless @handle_stat && @path_stat;
      return undef unless (($handle_stat[2] & S_IFMT) == S_IFREG);
      return undef unless (($path_stat[2] & S_IFMT) == S_IFREG);
      return undef unless "$handle_stat[0]:$handle_stat[1]" eq "$path_stat[0]:$path_stat[1]";
      return "$handle_stat[0]:$handle_stat[1]";
    }

    sub read_contents {
      my ($handle) = @_;
      seek($handle, 0, 0) or return undef;
      my $contents = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($handle, $chunk, 65536);
        return undef unless defined $count;
        last if $count == 0;
        $contents .= $chunk;
      }
      return $contents;
    }

    sub write_contents {
      my ($handle, $contents) = @_;
      seek($handle, 0, 0) or return 0;
      truncate($handle, 0) or return 0;
      my $offset = 0;
      my $length = length($contents);
      while ($offset < $length) {
        my $count = syswrite($handle, $contents, $length - $offset, $offset);
        return 0 unless defined($count) && $count > 0;
        $offset += $count;
      }
      return 1;
    }

    sub restore_existing {
      my ($handle, $name, $expected_identity, $contents) = @_;
      return 0 unless (regular_identity($handle, $name) || "") eq $expected_identity;
      return 0 unless write_contents($handle, $contents);
      my $restored_contents = read_contents($handle);
      return 0 unless defined($restored_contents) && $restored_contents eq $contents;
      return 0 unless (regular_identity($handle, $name) || "") eq $expected_identity;
      return 1;
    }

    sub remove_created {
      my ($handle, $name, $identity) = @_;
      return 0 unless defined($identity) && $identity ne "";
      return 0 unless (regular_identity($handle, $name) || "") eq $identity;
      return unlink($name) ? 1 : 0;
    }

    sub hash_contents {
      my ($contents) = @_;
      my ($read_handle, $write_handle);
      my $pid = eval {
        open2($read_handle, $write_handle, "git", "-C", $repo_root, "hash-object", "--stdin")
      };
      return undef unless defined $pid;
      binmode $read_handle;
      binmode $write_handle;
      my $offset = 0;
      my $length = length($contents);
      while ($offset < $length) {
        my $count = syswrite($write_handle, $contents, $length - $offset, $offset);
        return undef unless defined($count) && $count > 0;
        $offset += $count;
      }
      close $write_handle or return undef;
      my $hash = "";
      while (1) {
        my $chunk = "";
        my $count = sysread($read_handle, $chunk, 128);
        return undef unless defined $count;
        last if $count == 0;
        $hash .= $chunk;
      }
      close $read_handle or return undef;
      waitpid($pid, 0);
      return undef unless $? == 0;
      $hash =~ s/[\r\n]+\z//;
      return $hash if $hash =~ /\A[0-9a-f]+\z/;
      return undef;
    }

    regular_identity($source, $source_path) or die "generated hook must be a regular file\n";
    my $source_contents = read_contents($source);
    die "unable to read generated hook\n" unless defined $source_contents;
    regular_identity($source, $source_path) or die "generated hook changed while reading\n";

    my $hook;
    my $created = 0;
    my $original_contents = "";
    if ($expected_state eq "absent") {
      sysopen $hook, $hook_name, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_NONBLOCK, 0755
        or die "hook appeared during installation: $!\n";
      $created = 1;
    } else {
      sysopen $hook, $hook_name, O_RDWR | O_NOFOLLOW | O_NONBLOCK
        or die "unable to open existing hook: $!\n";
    }
    binmode $hook;
    flock($hook, LOCK_EX) or die "unable to lock hook: $!\n";
    my $identity = regular_identity($hook, $hook_name)
      or die "hook must be a regular file\n";
    if ($expected_state eq "absent") {
      die "hook appeared during installation\n" unless $created;
    } else {
      die "hook identity changed during installation\n" unless $identity eq $expected_identity;
      $original_contents = read_contents($hook);
      die "unable to read existing hook\n" unless defined $original_contents;
      my $original_hash = hash_contents($original_contents)
        or die "unable to hash existing hook\n";
      die "hook contents changed during installation\n" unless $original_hash eq $expected_hash;
      die "hook identity changed during installation\n"
        unless (regular_identity($hook, $hook_name) || "") eq $expected_identity;
      my $rechecked_contents = read_contents($hook);
      die "unable to read existing hook\n" unless defined $rechecked_contents;
      my $rechecked_hash = hash_contents($rechecked_contents)
        or die "unable to hash existing hook\n";
      die "hook contents changed during installation\n" unless $rechecked_hash eq $expected_hash;
      $original_contents = $rechecked_contents;
    }
    die "hook identity changed before installation\n"
      unless $expected_state eq "absent" || (regular_identity($hook, $hook_name) || "") eq $expected_identity;
    if (!write_contents($hook, $source_contents)) {
      if ($expected_state eq "absent") {
        remove_created($hook, $hook_name, $identity);
      } else {
        restore_existing($hook, $hook_name, $expected_identity, $original_contents);
      }
      die "unable to write hook\n";
    }
    my $written_contents = read_contents($hook);
    if (!defined($written_contents) || $written_contents ne $source_contents) {
      if ($expected_state eq "absent") {
        remove_created($hook, $hook_name, $identity);
      } else {
        restore_existing($hook, $hook_name, $expected_identity, $original_contents);
      }
      die "unable to verify hook\n";
    }
    if (!regular_identity($hook, $hook_name)) {
      die "hook identity changed after installation\n";
    }
    close $hook or die "unable to close hook: $!\n";
  ' "$repo_root" "$hook_path" "$source_path" "$expected_state" "$expected_identity" "$expected_hash"
}

cleanup_legacy_prepare_commit_msg() {
  local hook_path template_dir template_path hook_snapshot_value hook_identity

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
  hook_snapshot_value="$(hook_snapshot "$hook_path" 2>/dev/null || true)"
  hook_identity="${hook_snapshot_value%%$'\t'*}"
  if [ -f "$template_path" ] \
    && [ -n "$hook_identity" ] \
    && [ "$hook_snapshot_value" != "$hook_identity" ] \
    && hook_file_matches "$hook_path" "$template_path" \
    && safe_remove_legacy_hook "$hook_path" "$template_path" "$hook_identity"; then
      printf '[worktree-bootstrap] removed obsolete Lefthook prepare-commit-msg wrapper\n'
  else
    if [ -n "$hook_identity" ] && [ "$hook_snapshot_value" != "$hook_identity" ]; then
      printf '[worktree-bootstrap] prepare-commit-msg changed during migration; leaving it untouched\n' >&2
    else
      printf '[worktree-bootstrap] prepare-commit-msg is unmanaged; leaving it untouched\n' >&2
    fi
  fi
  rm -rf "$template_dir"
}

cleanup_legacy_prepare_commit_msg

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
  if [ -f "$template_path" ] && hook_file_matches "$existing_hook_path" "$template_path"; then
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
  if [ -f "$template_path" ] && hook_file_matches "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    [ "$hook_name" = 'pre-commit' ] && return 1
    return 0
  fi
  if [ "$hook_name" = 'pre-commit' ] \
    && legacy_template_path="$template_dir/pre-commit.legacy" \
    && cp "$template_path" "$legacy_template_path" \
    && customize_legacy_pre_commit_template "$legacy_template_path" \
    && [ -f "$legacy_template_path" ] \
    && hook_file_matches "$hook_path" "$legacy_template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  if [ "$hook_name" = 'pre-commit' ] \
    && customize_pre_commit_template "$template_path" \
    && [ -f "$template_path" ] \
    && hook_file_matches "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  if [ "$hook_name" = 'post-checkout' ] \
    && customize_post_checkout_template "$template_path" \
    && [ -f "$template_path" ] \
    && hook_file_matches "$hook_path" "$template_path"; then
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
  expected_hook_states=()
  expected_hook_identities=()
  expected_hook_hashes=()
  for hook_name in "${install_hooks[@]}"; do
    hook_path="$hooks_dir/$hook_name"
    if ! is_installable_hook "$hook_name" "$hook_path"; then
      printf '[worktree-bootstrap] %s changed during migration; leaving hooks untouched\n' "$hook_name" >&2
      exit 1
    fi
    hook_snapshot_value="$(hook_snapshot "$hook_path" 2>/dev/null || true)"
    hook_snapshot_identity="${hook_snapshot_value%%$'\t'*}"
    if [ -n "$hook_snapshot_value" ] && [ "$hook_snapshot_value" != "$hook_snapshot_identity" ]; then
      expected_hook_states+=('present')
      expected_hook_identities+=("$hook_snapshot_identity")
      expected_hook_hashes+=("${hook_snapshot_value#*$'\t'}")
    elif [ ! -e "$hook_path" ] && [ ! -L "$hook_path" ]; then
      expected_hook_states+=('absent')
      expected_hook_identities+=('')
      expected_hook_hashes+=('')
    else
      printf '[worktree-bootstrap] %s changed during migration; leaving hooks untouched\n' "$hook_name" >&2
      exit 1
    fi
  done
  install_template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")"
  if ! git -C "$install_template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$install_template_dir/lefthook.yml" \
    || ! (
    cd "$install_template_dir" &&
    "$lefthook_path" install "${install_hooks[@]}" >/dev/null 2>&1
  ); then
    rm -rf "$install_template_dir"
    printf '[worktree-bootstrap] could not generate hooks in a private directory\n' >&2
    exit 1
  fi

  for hook_name in "${install_hooks[@]}"; do
    source_hook_path="$install_template_dir/.git/hooks/$hook_name"
    if [ "$hook_name" = 'pre-commit' ] \
      && ! customize_pre_commit_template "$source_hook_path"; then
      rm -rf "$install_template_dir"
      printf '[worktree-bootstrap] could not generate pre-commit safety check\n' >&2
      exit 1
    fi
    if [ "$hook_name" = 'post-checkout' ] \
      && ! customize_post_checkout_template "$source_hook_path"; then
      rm -rf "$install_template_dir"
      printf '[worktree-bootstrap] could not generate post-checkout wrapper\n' >&2
      exit 1
    fi
    if ! [ -f "$source_hook_path" ] \
      || ! printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$source_hook_path"; then
      rm -rf "$install_template_dir"
      printf '[worktree-bootstrap] could not finalize generated %s hook\n' "$hook_name" >&2
      exit 1
    fi
  done

  hook_index=0
  if (
    for hook_name in "${install_hooks[@]}"; do
      hook_path="$hooks_dir/$hook_name"
      source_hook_path="$install_template_dir/.git/hooks/$hook_name"
      if ! safe_publish_hook \
        "$hook_path" \
        "$source_hook_path" \
        "${expected_hook_states[$hook_index]}" \
        "${expected_hook_identities[$hook_index]}" \
        "${expected_hook_hashes[$hook_index]}"; then
        printf '[worktree-bootstrap] %s changed during safe hook installation; leaving remaining hooks untouched\n' "$hook_name" >&2
        exit 1
      fi
      hook_index=$((hook_index + 1))
    done
  ); then
    install_status=0
  else
    install_status=$?
  fi
  rm -rf "$install_template_dir"
  exit "$install_status"
fi
