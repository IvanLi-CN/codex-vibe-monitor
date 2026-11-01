#!/usr/bin/env bash
set -euo pipefail

if [ "${LEFTHOOK:-}" = '0' ]; then
  exit 0
fi

repo_root="$(git rev-parse --show-toplevel)"
lock_path="$(git -C "$repo_root" rev-parse --git-path pre-commit.flock)"

exec perl -MFcntl=:flock - "$lock_path" "$repo_root" "$@" <<'EOF_PRE_COMMIT_LOCK'
use strict;
use warnings;

my ($lock_path, $repo_root, @hook_args) = @ARGV;
open my $lock, '>>', $lock_path or die "unable to open pre-commit lock: $!\n";
flock($lock, LOCK_EX) or die "unable to acquire pre-commit lock: $!\n";
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
exec {$lefthook} $lefthook, 'run', 'pre-commit', @hook_args;
die "unable to run Lefthook: $!\n";
EOF_PRE_COMMIT_LOCK
