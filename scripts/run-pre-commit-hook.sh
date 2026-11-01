#!/usr/bin/env bash
set -euo pipefail

case "${LEFTHOOK:-}" in
  0|false)
    exit 0
    ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
git_dir="$(git -C "$repo_root" rev-parse --absolute-git-dir)"
lock_path="$git_dir/pre-commit.flock"

exec perl -MCwd=abs_path -MFcntl=:DEFAULT -MFcntl=:flock -MFcntl=:mode - "$lock_path" "$repo_root" "$@" <<'EOF_PRE_COMMIT_LOCK'
use strict;
use warnings;

my ($lock_path, $repo_root, @hook_args) = @ARGV;
my $lock_separator = rindex($lock_path, '/');
die "unable to determine pre-commit lock directory\n" if $lock_separator < 1;
my $lock_directory = substr($lock_path, 0, $lock_separator);
my $lock_name = substr($lock_path, $lock_separator + 1);
my $expected_directory = abs_path($lock_directory)
  or die "unable to resolve pre-commit lock directory: $!\n";
sysopen my $directory, $lock_directory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW
  or die "unable to open pre-commit lock directory: $!\n";
flock($directory, LOCK_EX) or die "unable to acquire pre-commit lock directory: $!\n";
chdir $directory or die "unable to enter pre-commit lock directory: $!\n";
my $opened_directory = abs_path('.')
  or die "unable to resolve opened pre-commit lock directory: $!\n";
die "pre-commit lock directory changed while opening\n"
  unless $opened_directory eq $expected_directory;
die "pre-commit lock must not be a symlink\n" if -l $lock_name;
die "pre-commit lock must be a regular file\n" if -e $lock_name && !-f $lock_name;
sysopen my $lock, $lock_name, O_WRONLY | O_CREAT | O_APPEND | O_NOFOLLOW | O_NONBLOCK, 0600
  or die "unable to open pre-commit lock: $!\n";
sub verify_lock_identity {
    my ($handle, $name) = @_;
    my @handle_stat = stat($handle)
      or die "pre-commit lock must be a regular file\n";
    my @path_stat = lstat($name)
      or die "pre-commit lock must be a regular file\n";
    die "pre-commit lock must not be a symlink\n"
      if (($path_stat[2] & S_IFMT) == S_IFLNK);
    die "pre-commit lock must be a regular file\n"
      unless (($path_stat[2] & S_IFMT) == S_IFREG);
    die "pre-commit lock path changed while opening\n"
      unless "$handle_stat[0]:$handle_stat[1]" eq "$path_stat[0]:$path_stat[1]";
}
verify_lock_identity($lock, $lock_name);
flock($lock, LOCK_EX) or die "unable to acquire pre-commit lock: $!\n";
verify_lock_identity($lock, $lock_name);
chdir $repo_root or die "unable to enter repository: $!\n";

my $safety_status = system('bash', 'scripts/check-staged-formatter-safety.sh');
if ($safety_status == -1) {
    die "unable to run staged formatter safety check\n";
}
if ($safety_status & 127) {
    exit 1;
}
my $safety_code = $safety_status >> 8;
exit $safety_code if $safety_code != 0;

my $lefthook = $ENV{LEFTHOOK_BIN} || 'lefthook';
exec {$lefthook} $lefthook, 'run', 'pre-commit', '--no-auto-install', @hook_args;
die "unable to run Lefthook: $!\n";
EOF_PRE_COMMIT_LOCK
