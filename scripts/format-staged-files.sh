#!/usr/bin/env bash
set -euo pipefail

check_only=0
if [ "${1:-}" = '--check' ]; then
  check_only=1
  surface='all'
  shift
  candidate_files=()
  while IFS= read -r -d '' staged_file; do
    candidate_files+=("$staged_file")
  done < <(git diff --cached --name-only --diff-filter=ACMR -z)
else
  surface="${1:?formatter surface is required}"
  shift
  candidate_files=("$@")
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"
cd "$repo_root"
files=()

if [ "${#candidate_files[@]}" -gt 0 ]; then
  for file in "${candidate_files[@]}"; do
    # Preserve the pinned third-party snapshot and its upstream hash manifest.
    # Its two Rust patches are formatted by cargo fmt using the crate's edition.
    case "$file" in vendor/hotpath/*) continue ;; esac
    # Deleted paths are still present in Git's staged-file list, but cannot be formatted.
    case "$file" in
      /*|..|../*|*/..|*/../*) continue ;;
    esac
    path="$repo_root/$file"
    # Formatters follow symlinks, so no path component may escape the worktree.
    [ -L "$path" ] && continue
    [ -f "$path" ] || continue
    resolved_path="$(cd "$(dirname "$path")" && pwd -P)/$(basename "$path")"
    case "$resolved_path" in
      "$repo_root"/*) ;;
      *) continue ;;
    esac
    [ "$resolved_path" = "$path" ] || continue

    case "$surface" in
      web)
        case "$file" in
          web/*.js|web/*.ts|web/*.cjs|web/*.mjs|web/*.d.cts|web/*.d.mts|web/*.jsx|web/*.tsx|web/*.json|web/*.jsonc)
            files+=("$file")
            ;;
        esac
        ;;
      rust)
        case "$file" in *.rs) files+=("$file") ;; esac
        ;;
      markdown)
        case "$file" in *.md) files+=("$file") ;; esac
        ;;
      all)
        case "$file" in
          web/*.js|web/*.ts|web/*.cjs|web/*.mjs|web/*.d.cts|web/*.d.mts|web/*.jsx|web/*.tsx|web/*.json|web/*.jsonc|*.rs|*.md)
            files+=("$file")
            ;;
        esac
        ;;
    esac
  done
fi

deduplicated_files=()
if [ "${#files[@]}" -gt 0 ]; then
  for file in "${files[@]}"; do
    already_selected=0
    if [ "${#deduplicated_files[@]}" -gt 0 ]; then
      for selected_file in "${deduplicated_files[@]}"; do
        if [ "$selected_file" = "$file" ]; then
          already_selected=1
          break
        fi
      done
    fi
    if [ "$already_selected" -eq 0 ]; then
      deduplicated_files+=("$file")
    fi
  done
fi
if [ "${#deduplicated_files[@]}" -gt 0 ]; then
  files=("${deduplicated_files[@]}")
else
  files=()
fi

[ "${#files[@]}" -gt 0 ] || exit 0

for file in "${files[@]}"; do
  if ! git ls-files --stage --error-unmatch -- "$file" 2>/dev/null | awk '$3 == "0" { found = 1 } END { exit found ? 0 : 1 }'; then
    printf 'refusing to format a file that is not staged: %s\n' "$file" >&2
    exit 1
  fi
done

if [ "$check_only" -eq 1 ]; then
  for file in "${files[@]}"; do
    index_object="$(git rev-parse --verify ":$file")"
    worktree_object="$(git hash-object -- "$file")"
    if ! git diff --quiet -- "$file" || [ "$worktree_object" != "$index_object" ]; then
      printf 'refusing to auto-format partially staged file: %s\n' "$file" >&2
      printf 'stage or unstage all changes in this file before committing\n' >&2
      exit 1
    fi
  done
  exit 0
fi

original_modes=()
original_objects=()
original_worktree_hashes=()
original_assume_unchanged=()
original_skip_worktree=()
original_tmp_files=()
format_tmp_dir="$(mktemp -d "$repo_root/.codex-format.XXXXXX")"
format_tmp_files=()
index_path="$(git rev-parse --path-format=absolute --git-path index)"
original_index_snapshot="$format_tmp_dir/original-index"
original_index_identity=''
private_index_path=''
index_file_override=''
index_lock_path=''
index_lock_identity=''
index_lock_held=0
rollback_needed=0
rollback_after_index_install=0
worktree_written=()

index_identity() {
  local stat_identity index_hash

  [ -f "$index_path" ] || return 1
  [ ! -L "$index_path" ] || return 1
  if stat_identity="$(stat -c '%d:%i:%s:%Y' "$index_path" 2>/dev/null)"; then
    :
  else
    stat_identity="$(stat -f '%d:%i:%z:%m' "$index_path" 2>/dev/null)" || return 1
  fi
  index_hash="$(git hash-object -- "$index_path")" || return 1
  printf '%s:%s\n' "$stat_identity" "$index_hash"
}

capture_index_snapshot() {
  local snapshot_path="$1"
  local attempt before after

  for attempt in 1 2 3; do
    before="$(index_identity)" || return 1
    cp "$index_path" "$snapshot_path" || return 1
    after="$(index_identity)" || return 1
    if [ "$before" = "$after" ] && cmp -s "$index_path" "$snapshot_path"; then
      printf '%s\n' "$after"
      return 0
    fi
  done

  return 1
}

lock_identity() {
  local stat_identity

  [ -f "$index_lock_path" ] || return 1
  [ ! -L "$index_lock_path" ] || return 1
  if stat_identity="$(stat -c '%d:%i' "$index_lock_path" 2>/dev/null)"; then
    :
  else
    stat_identity="$(stat -f '%d:%i' "$index_lock_path" 2>/dev/null)" || return 1
  fi
  printf '%s\n' "$stat_identity"
}

index_lock_owned() {
  local current_identity

  [ "$index_lock_held" -eq 1 ] || return 1
  [ -n "$index_lock_identity" ] || return 1
  current_identity="$(lock_identity 2>/dev/null || true)"
  [ "$current_identity" = "$index_lock_identity" ]
}

index_matches_snapshot() {
  local snapshot_path="$1"
  local snapshot_identity="$2"
  local current_identity

  [ -n "$snapshot_identity" ] || return 1
  current_identity="$(index_identity 2>/dev/null || true)"
  [ "$current_identity" = "$snapshot_identity" ] \
    && cmp -s "$index_path" "$snapshot_path"
}

acquire_index_lock() {
  [ "$index_lock_held" -eq 0 ] || return 0
  index_lock_path="$index_path.lock"
  if ! index_lock_identity="$(perl -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -e '
    my ($path) = @ARGV;
    my $separator = rindex($path, "/");
    exit 1 if $separator < 1;
    my $directory_path = substr($path, 0, $separator);
    my $lock_name = substr($path, $separator + 1);
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 1;
    flock($directory, LOCK_EX) or exit 1;
    chdir $directory or exit 1;
    sysopen my $lock, $lock_name, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0666 or exit 1;
    my @stat = stat($lock) or do { unlink $lock_name; exit 1; };
    if (($stat[2] & S_IFMT) != S_IFREG) {
      unlink $lock_name;
      exit 1;
    }
    print "$stat[0]:$stat[1]\n";
    if (!close $lock) {
      unlink $lock_name;
      exit 1;
    }
  ' "$index_lock_path")"; then
    index_lock_path=''
    index_lock_identity=''
    return 1
  fi
  index_lock_held=1
}

remove_index_lock() {
  perl -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -e '
    my ($path, $expected_identity) = @ARGV;
    my $separator = rindex($path, "/");
    exit 2 if $separator < 1;
    my $directory_path = substr($path, 0, $separator);
    my $lock_name = substr($path, $separator + 1);
    sysopen my $directory, $directory_path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 2;
    flock($directory, LOCK_EX) or exit 2;
    chdir $directory or exit 2;
    sysopen my $lock, $lock_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK or exit 2;
    my @stat = stat($lock) or exit 2;
    exit 2 unless (($stat[2] & S_IFMT) == S_IFREG);
    exit 3 if "$stat[0]:$stat[1]" ne $expected_identity;
    my @path_stat = lstat($lock_name) or exit 2;
    exit 3 if "$path_stat[0]:$path_stat[1]" ne $expected_identity;
    unlink($lock_name) or exit 2;
  ' "$index_lock_path" "$index_lock_identity"
}

release_index_lock() {
  if [ "$index_lock_held" -eq 1 ]; then
    if index_lock_owned; then
      set +e
      remove_index_lock
      remove_status=$?
      set -e
      case "$remove_status" in
        0) ;;
        3) printf 'leaving a replaced Git index lock untouched\n' >&2 ;;
        *) printf 'unable to remove the formatter Git index lock\n' >&2 ;;
      esac
    else
      printf 'leaving a replaced Git index lock untouched\n' >&2
    fi
    index_lock_held=0
    index_lock_path=''
    index_lock_identity=''
  fi
}

update_index() {
  if [ -n "$index_file_override" ]; then
    GIT_INDEX_FILE="$index_file_override" git update-index "$@"
  else
    git update-index "$@"
  fi
}

write_worktree_files() {
  [ "$#" -gt 0 ] || return 0

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -MIPC::Open2 - "$repo_root" "$@" <<'EOF_WRITE_WORKTREE'
use strict;
use warnings;

my @arguments = @ARGV;
my $worktree_root = shift @arguments;
exit 2 if !defined $worktree_root || !@arguments || @arguments % 4;
my $resolved_worktree_root = abs_path($worktree_root) or exit 2;
my @records;

sub read_all {
  my ($handle) = @_;
  return undef unless seek($handle, 0, 0);
  return do { local $/; <$handle> };
}

sub hash_contents {
  my ($contents) = @_;
  my ($hash_reader, $hash_writer);
  my $hash_pid = open2($hash_reader, $hash_writer, 'git', 'hash-object', '--stdin');
  binmode $hash_reader;
  binmode $hash_writer;
  return undef unless print {$hash_writer} $contents;
  return undef unless close $hash_writer;
  my $hash = <$hash_reader>;
  return undef unless close $hash_reader;
  waitpid($hash_pid, 0);
  return undef if $? != 0 || !defined $hash;
  chomp $hash;
  return $hash;
}

sub write_all {
  my ($handle, $contents) = @_;
  my $offset = 0;
  while ($offset < length($contents)) {
    my $written = syswrite($handle, $contents, length($contents) - $offset, $offset);
    return 0 unless defined $written && $written > 0;
    $offset += $written;
  }
  return 1;
}

sub restore_partial_contents {
  my ($handle, $expected, $attempted) = @_;
  return 0 unless defined $expected;
  my $current = read_all($handle);
  return 0 unless defined $current;
  return 1 if $current eq $expected;
  return 0 unless length($current) <= length($attempted)
    && substr($attempted, 0, length($current)) eq $current;
  return 0 unless seek($handle, 0, 0);
  return 0 unless truncate($handle, 0);
  return 0 unless write_all($handle, $expected);
  return 0 unless seek($handle, 0, 0);
  my $restored = do { local $/; <$handle> };
  return defined($restored) && $restored eq $expected;
}

sub replace_contents {
  my ($handle, $contents, $expected) = @_;
  if (defined $expected) {
    my $current = read_all($handle);
    return 0 unless defined $current && $current eq $expected;
  }
  return 0 unless seek($handle, 0, 0);
  unless (truncate($handle, 0)) {
    restore_partial_contents($handle, $expected, $contents);
    return 0;
  }
  unless (write_all($handle, $contents)) {
    restore_partial_contents($handle, $expected, $contents);
    return 0;
  }
  unless (seek($handle, 0, 0)) {
    restore_partial_contents($handle, $expected, $contents);
    return 0;
  }
  my $actual = do { local $/; <$handle> };
  return 1 if defined($actual) && $actual eq $contents;
  restore_partial_contents($handle, $expected, $contents);
  return 0;
}

sub restore_written {
  my ($records) = @_;
  my $failed = 0;
  for my $record (@{$records}) {
    next unless $record->{written};
    my $actual = read_all($record->{target});
    if (!defined $actual) {
      $failed = 1;
      next;
    }
    if ($actual eq $record->{before}) {
      next;
    }
    if ($actual eq $record->{desired}) {
      $failed = 1 unless replace_contents($record->{target}, $record->{before}, $record->{desired});
    } else {
      $failed = 1;
    }
  }
  return !$failed;
}

for (my $offset = 0; $offset < @arguments; $offset += 4) {
  my ($target_path, $expected_hash, $desired_hash, $source_path) = @arguments[$offset .. $offset + 3];
  my $root_prefix = $resolved_worktree_root eq '/' ? '/' : "$resolved_worktree_root/";
  exit 3 unless index($target_path, $root_prefix) == 0;
  my $relative_target = substr($target_path, length($root_prefix));
  my @target_components = split('/', $relative_target, -1);
  exit 2 if !@target_components || grep { $_ eq '' || $_ eq '.' || $_ eq '..' } @target_components;
  my $target_name = pop @target_components;
  my @directory_handles;
  sysopen my $root_handle, '/', O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 2;
  push @directory_handles, $root_handle;
  chdir $root_handle or exit 2;
  for my $component (grep { length $_ } split('/', $resolved_worktree_root)) {
    exit 2 if $component eq '.' || $component eq '..';
    sysopen my $next_directory, $component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 2;
    chdir $next_directory or exit 2;
    push @directory_handles, $next_directory;
  }
  for my $component (@target_components) {
    sysopen my $next_directory, $component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 2;
    chdir $next_directory or exit 2;
    push @directory_handles, $next_directory;
  }
  sysopen my $target, $target_name, O_RDWR | O_NOFOLLOW | O_NONBLOCK or exit 2;
  binmode $target;
  my @target_stat = stat($target) or exit 2;
  exit 3 unless (($target_stat[2] & S_IFMT) == S_IFREG);
  flock($target, LOCK_EX) or exit 2;
  sysopen my $source, $source_path, O_RDONLY | O_NOFOLLOW or exit 2;
  binmode $source;

  my $before = read_all($target);
  my $desired = read_all($source);
  exit 2 unless defined $before && defined $desired;
  my $before_hash = hash_contents($before);
  my $source_hash = hash_contents($desired);
  exit 2 unless defined $before_hash && defined $source_hash;
  exit 3 if $source_hash ne $desired_hash;
  exit 3 if $before_hash ne $expected_hash && $before_hash ne $desired_hash;
  push @records, {
    before => $before,
    desired => $desired,
    directories => \@directory_handles,
    needs_write => $before_hash eq $expected_hash,
    source => $source,
    target => $target,
    written => 0,
  };
}

for my $record (@records) {
  next unless $record->{needs_write};
  $record->{written} = 1;
  if (!replace_contents($record->{target}, $record->{desired}, $record->{before})) {
    my $restored = restore_written(\@records);
    exit($restored ? 2 : 4);
  }
}

for my $record (@records) {
  close $record->{source} or exit 2;
  close $record->{target} or exit 2;
  for my $directory (@{$record->{directories}}) {
    close $directory or exit 2;
  }
}
EOF_WRITE_WORKTREE
}

install_private_index() {
  local source="$1"
  local lock_path="$2"
  local index_path_arg="$3"
  local lock_identity_arg="$4"
  local expected_index_identity_arg="$5"
  local repo_root_arg="$6"
  local original_index_snapshot_arg="$7"
  shift 7

  perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode -MIPC::Open2 - "$source" "$lock_path" "$index_path_arg" "$lock_identity_arg" "$expected_index_identity_arg" "$repo_root_arg" "$original_index_snapshot_arg" "$@" <<'EOF_INSTALL_INDEX'
use strict;
use warnings;

my ($source_path, $lock_path, $index_path, $expected_lock_identity,
    $expected_index_identity, $worktree_root, $original_index_snapshot,
    @target_arguments) = @ARGV;
exit 2 if @target_arguments % 2;
my $lock_separator = rindex($lock_path, '/');
my $index_separator = rindex($index_path, '/');
exit 2 if $lock_separator < 1 || $index_separator < 1;
my $lock_directory = substr($lock_path, 0, $lock_separator);
my $index_directory = substr($index_path, 0, $index_separator);
my $lock_name = substr($lock_path, $lock_separator + 1);
my $index_name = substr($index_path, $index_separator + 1);
my @expected_index_parts = split(/:/, $expected_index_identity);
exit 2 if @expected_index_parts < 5;
my $expected_index_device = $expected_index_parts[0];
my $expected_index_inode = $expected_index_parts[1];
my $expected_index_hash = $expected_index_parts[-1];
my $expected_directory = abs_path($lock_directory) or exit 2;
my $resolved_index_directory = abs_path($index_directory) or exit 2;
exit 3 if $resolved_index_directory ne $expected_directory;
sysopen my $directory, $lock_directory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or exit 2;
flock($directory, LOCK_EX) or exit 2;
chdir $directory or exit 2;
my $opened_directory = abs_path('.') or exit 2;
exit 3 if $opened_directory ne $expected_directory;
sysopen my $lock, $lock_name, O_WRONLY | O_NOFOLLOW | O_NONBLOCK or exit 2;
my @lock_stat = stat($lock) or exit 2;
exit 2 unless (($lock_stat[2] & S_IFMT) == S_IFREG);
my $lock_identity = "$lock_stat[0]:$lock_stat[1]";
exit 3 if $lock_identity ne $expected_lock_identity;
my @lock_path_stat = lstat($lock_name) or exit 2;
exit 3 if "$lock_path_stat[0]:$lock_path_stat[1]" ne $expected_lock_identity;
sub read_all {
  my ($handle) = @_;
  return undef unless seek($handle, 0, 0);
  return do { local $/; <$handle> };
}

sub hash_contents {
  my ($contents) = @_;
  return undef unless defined $contents;
  my ($hash_reader, $hash_writer);
  my $hash_pid = open2($hash_reader, $hash_writer, 'git', 'hash-object', '--stdin');
  binmode $hash_reader;
  binmode $hash_writer;
  return undef unless print {$hash_writer} $contents;
  return undef unless close $hash_writer;
  my $hash = <$hash_reader>;
  return undef unless close $hash_reader;
  waitpid($hash_pid, 0);
  return undef if $? != 0 || !defined $hash;
  chomp $hash;
  return $hash;
}

sub write_all {
  my ($handle, $contents) = @_;
  my $offset = 0;
  while ($offset < length($contents)) {
    my $written = syswrite($handle, $contents, length($contents) - $offset, $offset);
    return 0 unless defined $written && $written > 0;
    $offset += $written;
  }
  return 1;
}

my $resolved_worktree_root = abs_path($worktree_root) or exit 2;

sub open_worktree_target {
  my ($path, $expected_hash) = @_;
  my $root_prefix = $resolved_worktree_root eq '/' ? '/' : "$resolved_worktree_root/";
  return unless index($path, $root_prefix) == 0;
  my $relative_path = substr($path, length($root_prefix));
  my @components = split('/', $relative_path, -1);
  return if !@components || grep { $_ eq '' || $_ eq '.' || $_ eq '..' } @components;
  my $name = pop @components;
  my $directory;
  sysopen $directory, '/', O_RDONLY | O_DIRECTORY | O_NOFOLLOW or return;
  chdir $directory or return;
  for my $component (grep { length $_ } split('/', $resolved_worktree_root)) {
    return if $component eq '.' || $component eq '..';
    my $next_directory;
    sysopen $next_directory, $component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or return;
    chdir $next_directory or return;
    close $directory or return;
    $directory = $next_directory;
  }
  for my $component (@components) {
    my $next_directory;
    sysopen $next_directory, $component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW or return;
    chdir $next_directory or return;
    close $directory or return;
    $directory = $next_directory;
  }
  sysopen my $target, $name, O_RDWR | O_NOFOLLOW | O_NONBLOCK or return;
  binmode $target;
  my @target_stat = stat($target) or return;
  return unless (($target_stat[2] & S_IFMT) == S_IFREG);
  flock($target, LOCK_EX) or return;
  return ($target, $directory, $expected_hash);
}

sub verify_index_path {
  my @path_stat = lstat($index_name) or return 0;
  return 0 unless (($path_stat[2] & S_IFMT) == S_IFREG);
  return 0 unless "$path_stat[0]:$path_stat[1]" eq "$expected_index_device:$expected_index_inode";
  sysopen my $index, $index_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK or return 0;
  binmode $index;
  my @fd_stat = stat($index) or return 0;
  my $contents = read_all($index);
  close $index or return 0;
  my @after_stat = lstat($index_name) or return 0;
  return 0 unless "$fd_stat[0]:$fd_stat[1]" eq "$after_stat[0]:$after_stat[1]";
  my $actual_hash = hash_contents($contents);
  return defined($actual_hash) && $actual_hash eq $expected_index_hash;
}

sub verify_targets {
  my ($targets) = @_;
  for my $target (@{$targets}) {
    my $contents = read_all($target->{handle});
    my $actual_hash = hash_contents($contents);
    return 0 unless defined($actual_hash) && $actual_hash eq $target->{hash};
  }
  return 1;
}

sub restore_index {
  my ($snapshot_path, $lock_name, $index_name, $installed_identity) = @_;
  my @current_stat = lstat($index_name) or return 0;
  return 0 unless "$current_stat[0]:$current_stat[1]" eq $installed_identity;
  sysopen my $restore_lock, $lock_name,
    O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_NONBLOCK, 0666 or return 0;
  binmode $restore_lock;
  sysopen my $snapshot, $snapshot_path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK
    or return 0;
  binmode $snapshot;
  my $contents = read_all($snapshot);
  close $snapshot or return 0;
  return 0 unless defined $contents;
  truncate($restore_lock, 0) or return 0;
  write_all($restore_lock, $contents) or return 0;
  close $restore_lock or return 0;
  my @restore_stat = lstat($lock_name) or return 0;
  my $restore_identity = "$restore_stat[0]:$restore_stat[1]";
  my @before_rename = lstat($index_name) or return 0;
  return 0 unless "$before_rename[0]:$before_rename[1]" eq $installed_identity;
  return rename($lock_name, $index_name) ? 1 : 0;
}

my @targets;
for (my $offset = 0; $offset < @target_arguments; $offset += 2) {
  my ($path, $expected_hash) = @target_arguments[$offset .. $offset + 1];
  my ($target, $target_directory, $hash) = open_worktree_target($path, $expected_hash);
  exit 3 unless defined $target && defined $target_directory;
  push @targets, { handle => $target, directory => $target_directory, hash => $hash };
}
chdir $directory or exit 2;
exit 3 unless verify_index_path();
exit 3 unless verify_targets(\@targets);
sysopen my $source, $source_path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK or exit 2;
binmode $source;
my $contents = read_all($source);
close $source or exit 2;
defined $contents or exit 2;
binmode $lock;
truncate($lock, 0) or exit 2;
write_all($lock, $contents) or exit 2;
my @after_lock_stat = lstat($lock_name) or exit 2;
exit 3 if "$after_lock_stat[0]:$after_lock_stat[1]" ne $expected_lock_identity;
exit 3 unless verify_index_path();
exit 3 unless verify_targets(\@targets);
exit 3 unless verify_index_path();
rename($lock_name, $index_name) or exit 2;
my @installed_stat = lstat($index_name) or exit 4;
my $installed_identity = "$installed_stat[0]:$installed_stat[1]";
if (!verify_targets(\@targets)) {
  my $restored = restore_index($original_index_snapshot, $lock_name, $index_name, $installed_identity);
  exit($restored ? 4 : 5);
}
close $lock or exit 2;
for my $target (@targets) {
  close $target->{handle} or exit 2;
  close $target->{directory} or exit 2;
}
EOF_INSTALL_INDEX
}

cleanup_format_tmp() {
  local exit_code=$?

  trap - EXIT INT TERM
  if [ "$rollback_needed" -eq 1 ] && declare -F restore_original_state >/dev/null 2>&1; then
    set +e
    restore_original_state
    set -e
  fi
  release_index_lock
  rm -rf "$format_tmp_dir"
  exit "$exit_code"
}
trap cleanup_format_tmp EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if ! original_index_identity="$(capture_index_snapshot "$original_index_snapshot")"; then
  printf 'refusing to format while the Git index is changing\n' >&2
  exit 1
fi

for file in "${files[@]}"; do
  if ! git diff --quiet -- "$file"; then
    printf 'refusing to auto-format partially staged file: %s\n' "$file" >&2
    printf 'stage or unstage all changes in this file before committing\n' >&2
    exit 1
  fi
  index_entry="$(git ls-files --stage --error-unmatch -- "$file" | awk '$3 == "0" { print; exit }')"
  read -r original_mode original_object _ _ <<< "$index_entry"
  original_worktree_hash="$(git hash-object -- "$file")"
  if [ "$original_worktree_hash" != "$original_object" ]; then
    printf 'refusing to auto-format partially staged file: %s\n' "$file" >&2
    printf 'stage or unstage all changes in this file before committing\n' >&2
    exit 1
  fi
  index_state="$(git ls-files -v -- "$file" | cut -c1)"
  original_modes+=("$original_mode")
  original_objects+=("$original_object")
  case "$index_state" in
    [a-z]) original_assume_unchanged+=(1) ;;
    *) original_assume_unchanged+=(0) ;;
  esac
  case "$index_state" in
    [Ss]) original_skip_worktree+=(1) ;;
    *) original_skip_worktree+=(0) ;;
  esac
  original_worktree_hashes+=("$original_worktree_hash")
  format_tmp_file="$format_tmp_dir/$file"
  original_tmp_file="$format_tmp_dir/.original-${#format_tmp_files[@]}"
  mkdir -p "$(dirname "$format_tmp_file")"
  git cat-file blob "$original_object" > "$format_tmp_file"
  git cat-file blob "$original_object" > "$original_tmp_file"
  original_tmp_files+=("$original_tmp_file")
  format_tmp_files+=("$format_tmp_file")
done

is_safe_worktree_path() {
  local file="$1"
  local worktree_path resolved_path

  worktree_path="$repo_root/$file"
  if [ -L "$worktree_path" ] || [ ! -f "$worktree_path" ]; then
    return 1
  fi
  resolved_path="$(cd "$(dirname "$worktree_path")" && pwd -P)/$(basename "$worktree_path")"
  [ "$resolved_path" = "$worktree_path" ]
}

case "$surface" in
  web)
    biome_bin="${CODEX_HOOK_BIOME_BIN:-$repo_root/node_modules/.bin/biome}"
    for index in "${!files[@]}"; do
      file="${files[$index]}"
      format_tmp_file="${format_tmp_files[$index]}"
      format_output="$format_tmp_dir/.biome-output-$index"
      "$biome_bin" check --write --stdin-file-path="$file" < "$format_tmp_file" > "$format_output"
      mv "$format_output" "$format_tmp_file"
    done
    ;;
  rust)
    rustfmt_bin="${CODEX_HOOK_RUSTFMT_BIN:-rustfmt}"
    "$rustfmt_bin" --edition 2024 --config skip_children=true -- "${format_tmp_files[@]}"
    ;;
  markdown)
    dprint_bin="${CODEX_HOOK_DPRINT_BIN:-$repo_root/node_modules/.bin/dprint}"
    for index in "${!files[@]}"; do
      file="${files[$index]}"
      format_tmp_file="${format_tmp_files[$index]}"
      format_output="$format_tmp_dir/.dprint-output-$index"
      "$dprint_bin" fmt --stdin "$file" < "$format_tmp_file" > "$format_output"
      mv "$format_output" "$format_tmp_file"
    done
    ;;
  *)
    printf 'unknown formatter surface: %s\n' "$surface" >&2
    exit 2
    ;;
esac

formatted_hashes=()
format_patch_file="$format_tmp_dir/formatted.patch"
: > "$format_patch_file"
for index in "${!files[@]}"; do
  file="${files[$index]}"
  format_tmp_file="${format_tmp_files[$index]}"
  worktree_path="$repo_root/$file"
  if ! is_safe_worktree_path "$file"; then
    printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    exit 1
  fi
  current_index="$(git rev-parse --verify ":$file")"
  current_worktree_hash="$(git hash-object -- "$file")"
  if [ "$current_index" != "${original_objects[$index]}" ] \
    || [ "$current_worktree_hash" != "${original_worktree_hashes[$index]}" ]; then
    printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    exit 1
  fi

  formatted_hash="$(git hash-object -- "$format_tmp_file")"
  formatted_hashes+=("$formatted_hash")
  if [ "$formatted_hash" = "${original_worktree_hashes[$index]}" ]; then
    continue
  fi

  context_lines="$(wc -l < "$file")"
  formatted_lines="$(wc -l < "$format_tmp_file")"
  if [ "$formatted_lines" -gt "$context_lines" ]; then
    context_lines="$formatted_lines"
  fi
  context_lines=$((context_lines + 1))
  set +e
  diff -u -U "$context_lines" -L "a/$file" -L "b/$file" "$worktree_path" "$format_tmp_file" >> "$format_patch_file"
  diff_status=$?
  set -e
  if [ "$diff_status" -ne 1 ]; then
    printf 'unable to create formatter patch for: %s\n' "$file" >&2
    exit 1
  fi
done

# Restore only worktree files written by this formatter while its index lock is held.
restore_index_metadata() {
  local index="$1"
  local file="${files[$index]}"

  if [ "${original_assume_unchanged[$index]}" -eq 1 ]; then
    update_index --assume-unchanged -- "$file"
  else
    update_index --no-assume-unchanged -- "$file"
  fi
  if [ "${original_skip_worktree[$index]}" -eq 1 ]; then
    update_index --skip-worktree -- "$file"
  else
    update_index --no-skip-worktree -- "$file"
  fi
}

restore_original_state() {
  local index rollback_failed=0
  local -a rollback_args=()

  if { ! index_lock_owned && [ "$rollback_after_index_install" -ne 1 ]; } || [ -z "$private_index_path" ]; then
    printf 'refusing to roll back formatter state without the formatter index lock\n' >&2
    return 1
  fi
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to roll back formatter state after a concurrent Git index change\n' >&2
    release_index_lock
    return 1
  fi

  for index in "${!files[@]}"; do
    [ "${worktree_written[$index]:-0}" -eq 1 ] || continue
    rollback_args+=(
      "$repo_root/${files[$index]}"
      "${formatted_hashes[$index]}"
      "${original_worktree_hashes[$index]}"
      "${original_tmp_files[$index]}"
    )
  done
  if [ "${#rollback_args[@]}" -gt 0 ] && ! write_worktree_files "${rollback_args[@]}"; then
    rollback_failed=1
  fi

  if [ "$rollback_failed" -ne 0 ]; then
    printf 'unable to fully roll back formatter worktree changes\n' >&2
    release_index_lock
    return 1
  fi

  release_index_lock
}

if [ -s "$format_patch_file" ]; then
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to stage concurrently modified files: Git index changed during formatting\n' >&2
    exit 1
  fi
  if ! acquire_index_lock; then
    printf 'refusing to stage while the Git index is concurrently locked\n' >&2
    exit 1
  fi
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to stage concurrently modified files: Git index changed during formatting\n' >&2
    release_index_lock
    exit 1
  fi

  private_index_path="$format_tmp_dir/formatted-index"
  if ! cp "$original_index_snapshot" "$private_index_path"; then
    printf 'unable to prepare the private Git index for formatting\n' >&2
    release_index_lock
    exit 1
  fi
  index_file_override="$private_index_path"
  if ! GIT_INDEX_FILE="$private_index_path" git apply --check --cached --binary "$format_patch_file"; then
    index_file_override=''
    private_index_path=''
    release_index_lock
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi
  for index in "${!files[@]}"; do
    file="${files[$index]}"
    if ! is_safe_worktree_path "$file" \
      || [ "$(git hash-object -- "$file")" != "${original_worktree_hashes[$index]}" ]; then
      for concurrent_file in "${files[@]}"; do
        printf 'refusing to stage concurrently modified file: %s\n' "$concurrent_file" >&2
      done
      index_file_override=''
      private_index_path=''
      release_index_lock
      exit 1
    fi
  done
  rollback_needed=1
  if ! GIT_INDEX_FILE="$private_index_path" git apply --cached --binary "$format_patch_file"; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi

  for index in "${!files[@]}"; do
    restore_index_metadata "$index"
  done

  forward_args=()
  for index in "${!files[@]}"; do
    [ "${formatted_hashes[$index]}" != "${original_worktree_hashes[$index]}" ] || continue
    worktree_written[$index]=1
    forward_args+=(
      "$repo_root/${files[$index]}"
      "${original_worktree_hashes[$index]}"
      "${formatted_hashes[$index]}"
      "${format_tmp_files[$index]}"
    )
  done
  if ! write_worktree_files "${forward_args[@]}"; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    for concurrent_file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$concurrent_file" >&2
    done
    exit 1
  fi

  verification_failed=0
  for index in "${!files[@]}"; do
    file="${files[$index]}"
    if ! is_safe_worktree_path "$file" \
      || [ "$(git hash-object -- "$file")" != "${formatted_hashes[$index]}" ] \
      || [ "$(GIT_INDEX_FILE="$private_index_path" git rev-parse --verify ":$file")" != "${formatted_hashes[$index]}" ]; then
      verification_failed=1
    fi
  done

  if [ "$verification_failed" -ne 0 ]; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi

  if ! index_lock_owned; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    printf 'refusing to install the formatted Git index after losing lock ownership\n' >&2
    exit 1
  fi
  install_args=()
  for index in "${!files[@]}"; do
    [ "${formatted_hashes[$index]}" != "${original_worktree_hashes[$index]}" ] || continue
    install_args+=(
      "$repo_root/${files[$index]}"
      "${formatted_hashes[$index]}"
    )
  done
  commit_signal_status=0
  trap 'commit_signal_status=130' INT
  trap 'commit_signal_status=143' TERM
  install_index_args=(
    "$private_index_path"
    "$index_lock_path"
    "$index_path"
    "$index_lock_identity"
    "$original_index_identity"
    "$repo_root"
    "$original_index_snapshot"
  )
  if [ "${#install_args[@]}" -gt 0 ]; then
    install_index_args+=("${install_args[@]}")
  fi
  install_status=0
  if install_private_index "${install_index_args[@]}"; then
    :
  else
    install_status=$?
    trap 'exit 130' INT
    trap 'exit 143' TERM
    if [ "$install_status" -eq 4 ]; then
      index_lock_held=0
      index_lock_path=''
      index_lock_identity=''
      rollback_after_index_install=1
    fi
    restore_original_state || true
    rollback_after_index_install=0
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    printf 'unable to install the formatted Git index\n' >&2
    exit 1
  fi
  index_lock_held=0
  index_lock_path=''
  index_lock_identity=''
  rollback_after_index_install=0
  index_file_override=''
  private_index_path=''
  rollback_needed=0
  trap 'exit 130' INT
  trap 'exit 143' TERM
  if [ "$commit_signal_status" -ne 0 ]; then
    exit "$commit_signal_status"
  fi
fi
