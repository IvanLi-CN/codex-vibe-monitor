#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dockerfile="$repo_root/Dockerfile"
runner="$repo_root/.github/scripts/run-backend-tests.sh"
compose_file="$repo_root/compose.backend-test.yml"
ci_main_workflow="$repo_root/.github/workflows/ci-main.yml"
backend_test_image_workflow="$repo_root/.github/workflows/backend-test-image.yml"
release_workflow="$repo_root/.github/workflows/release.yml"
tmp_dir="$(mktemp -d)"
partition_workspace="/tmp/codex-vibe-monitor-backend-test-contract-${PPID}"
trap 'rm -rf "${tmp_dir}" "${partition_workspace}"' EXIT

backend_test_stage="$(sed -n '/^FROM rust:1.96.0-bookworm AS backend-test$/,/^# Stage 8:/p' "$dockerfile")"
grep -q '^FROM rust:1.96.0-bookworm AS backend-test$' "$dockerfile"
grep -q '^  backend-test:$' "$compose_file"
grep -q 'target: backend-test' "$compose_file"
grep -q 'CARGO_NEXTEST_VERSION=0.9.138' "$dockerfile"
grep -q 'CARGO_NEXTEST_SHA256_AMD64=3793bf0c27607b196f502c39b2108f571de89fcda7586ae6beefa11ee177b216' "$dockerfile"
grep -q 'rustup component add clippy rustfmt' "$dockerfile"
grep -q 'libsqlite3-dev zstd python3' "$dockerfile"
grep -q 'install -m 0755 /tmp/cargo-nextest /usr/local/cargo/bin/cargo-nextest' "$dockerfile"
grep -q '^COPY scripts/search-raw ./scripts/search-raw$' "$dockerfile"
grep -q '^RUN mkdir -p target \\$' "$dockerfile"
if grep -Eq '/codex-scratch|/srv/app/data|CARGO_TARGET_DIR=' <<<"$backend_test_stage"; then
  echo 'backend-test image must not encode runner-private writable paths' >&2
  exit 1
fi
grep -q '^    && chown 65534:65534 target$' "$dockerfile"
grep -q 'ENTRYPOINT \["bash", ".github/scripts/run-backend-tests.sh"\]' "$dockerfile"
grep -q '^    entrypoint: \[\]$' "$compose_file"
grep -q '^    command: \["sleep", "infinity"\]$' "$compose_file"
grep -q '^    user: "65534:65534"$' "$compose_file"
if grep -Fq 'CARGO_HOME:' "$compose_file"; then
  echo 'Compose must not fix the project Cargo home path' >&2
  exit 1
fi

web_builder_section="$(sed -n '/^FROM oven\/bun:.* AS web-builder$/,/^# Stage 2:/p' "$dockerfile")"
web_arg_line="$(grep -n '^ARG APP_EFFECTIVE_VERSION$' <<<"$web_builder_section" | cut -d: -f1)"
web_workdir_line="$(grep -n '^WORKDIR /app/web$' <<<"$web_builder_section" | cut -d: -f1)"
if (( web_arg_line < web_workdir_line )); then
  echo 'web builder must not declare APP_EFFECTIVE_VERSION before dependency installation' >&2
  exit 1
fi
web_copy_line="$(grep -n '^COPY web/ \.\/$' <<<"$web_builder_section" | cut -d: -f1)"
if (( web_arg_line != web_copy_line + 1 )); then
  echo 'web builder must declare APP_EFFECTIVE_VERSION immediately before version-dependent build steps' >&2
  exit 1
fi

rust_builder_section="$(sed -n '/^FROM rust:.* AS rust-builder$/,/^# Stage 3:/p' "$dockerfile")"
rust_arg_line="$(grep -n '^ARG APP_EFFECTIVE_VERSION$' <<<"$rust_builder_section" | cut -d: -f1)"
rust_workdir_line="$(grep -n '^WORKDIR /app$' <<<"$rust_builder_section" | cut -d: -f1)"
if (( rust_arg_line < rust_workdir_line )); then
  echo 'rust builder must not declare APP_EFFECTIVE_VERSION before dependency installation' >&2
  exit 1
fi
rust_copy_line="$(grep -n '^COPY src \.\/src$' <<<"$rust_builder_section" | cut -d: -f1)"
if (( rust_arg_line != rust_copy_line + 1 )); then
  echo 'rust builder must declare APP_EFFECTIVE_VERSION immediately before source build steps' >&2
  exit 1
fi
grep -q 'org.opencontainers.image.revision=${APP_GIT_REVISION}' "$dockerfile"

set +e
invalid_partition_output="$(PATH=/usr/bin:/bin bash "$runner" --profile stateful-sqlite --partition hash:3/2 2>&1)"
invalid_partition_rc=$?
set -e
[[ "$invalid_partition_rc" == 64 ]]
grep -q -- '--partition requires 1 <= N <= M' <<<"$invalid_partition_output"

contract_bin="$tmp_dir/contract-bin"
contract_record="$tmp_dir/contract-record"
tmp_root="$(cd "$tmp_dir" && pwd -P)"
default_workspace="$tmp_root/arbitrary-workspace"
external_cargo_home="$tmp_root/external-cargo-home"
external_target_dir="$tmp_root/external-target"
mkdir -p "$contract_bin"
cat >"$contract_bin/cargo-nextest" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$contract_bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'cargo_home=%s\n' "${CARGO_HOME:-}" >>"${BACKEND_CONTRACT_RECORD:?}"
printf 'cargo_target_dir=%s\n' "${CARGO_TARGET_DIR:-}" >>"${BACKEND_CONTRACT_RECORD:?}"
printf 'cargo_net_offline=%s\n' "${CARGO_NET_OFFLINE:-}" >>"${BACKEND_CONTRACT_RECORD:?}"
printf '%q ' "$@" >>"${BACKEND_CONTRACT_RECORD:?}"
printf '\n' >>"${BACKEND_CONTRACT_RECORD:?}"
printf '%q ' "$@"
printf '\n'
EOF
chmod +x "$contract_bin/cargo-nextest" "$contract_bin/cargo"
partition_output="$(PATH="$contract_bin:/usr/bin:/bin" BACKEND_TEST_WORKSPACE="$partition_workspace" BACKEND_CONTRACT_RECORD="$contract_record" bash "$runner" --profile stateful-sqlite --partition hash:1/2 2>&1)"
grep -q -- '--partition hash:1/2' <<<"$partition_output"
grep -q 'backend_test_cache_mode=ephemeral' <<<"$partition_output"

: >"$contract_record"
default_output="$(env -u CARGO_NET_OFFLINE CARGO_HOME= CARGO_TARGET_DIR= \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight 2>&1)"
grep -q 'backend_test_network_mode=online' <<<"$default_output"
grep -q "cargo_home=$default_workspace/cargo-home" "$contract_record"
grep -q "cargo_target_dir=$default_workspace/target" "$contract_record"
[[ -d "$default_workspace/cargo-home" && -d "$default_workspace/target" ]]

valid_dot_workspace="$tmp_root/cache..v2"
dot_output="$(env -u CARGO_NET_OFFLINE CARGO_HOME= CARGO_TARGET_DIR= \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$valid_dot_workspace" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight 2>&1)"
grep -q 'backend_test_cache_mode=ephemeral' <<<"$dot_output"

: >"$contract_record"
external_output="$(env -u CARGO_NET_OFFLINE \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME="$external_cargo_home" \
  CARGO_TARGET_DIR="$external_target_dir" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight 2>&1)"
grep -q 'backend_test_cache_mode=external' <<<"$external_output"
grep -q "cargo_home=$external_cargo_home" "$contract_record"
grep -q "cargo_target_dir=$external_target_dir" "$contract_record"

trailing_output="$(env -u CARGO_NET_OFFLINE \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace/" \
  CARGO_HOME="$external_cargo_home/" \
  CARGO_TARGET_DIR="$external_target_dir/" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight 2>&1)"
grep -q 'backend_test_cache_mode=external' <<<"$trailing_output"

: >"$contract_record"
offline_output="$(env CARGO_NET_OFFLINE=true \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME="$external_cargo_home" \
  CARGO_TARGET_DIR="$external_target_dir" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight 2>&1)"
grep -q 'backend_test_network_mode=offline' <<<"$offline_output"
grep -q '^cargo_net_offline=true$' "$contract_record"

expect_failure() {
  local expected="$1"
  shift
  set +e
  "$@" >/dev/null 2>&1
  local status="$?"
  set -e
  [[ "$status" == "$expected" ]] || {
    echo "expected exit $expected, got $status" >&2
    exit 1
  }
}
symlink_workspace="$tmp_root/symlink-workspace"
mkdir -p "$symlink_workspace"
ln -s "$repo_root" "$symlink_workspace/cargo-home"
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$symlink_workspace" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
ln -s "$repo_root/does-not-exist" "$symlink_workspace/cargo-target-link"
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_TARGET_DIR="$symlink_workspace/cargo-target-link/nested" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight --partition hash:1/2
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE=/ \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME="$tmp_root/cache//nested" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$tmp_dir/../escaped" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME=relative \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_TARGET_DIR="$default_workspace/inside" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME="$external_cargo_home" \
  CARGO_TARGET_DIR="$external_cargo_home" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$repo_root" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight
expect_failure 64 env \
  PATH="$contract_bin:/usr/bin:/bin" \
  BACKEND_TEST_WORKSPACE="$default_workspace" \
  CARGO_HOME="$repo_root" \
  CARGO_TARGET_DIR="$external_target_dir" \
  BACKEND_CONTRACT_RECORD="$contract_record" \
  bash "$runner" --profile lightweight

if grep -Fq '^  backend-test-image:' "$ci_main_workflow"; then
  echo 'backend-test image must be outside the CI Main completion path' >&2
  exit 1
fi

if ! grep -Fq 'workflow_run:' "$backend_test_image_workflow" || ! grep -Fq 'github.event.workflow_run.conclusion == '\''success'\''' "$backend_test_image_workflow"; then
  echo 'backend-test image workflow must run only after successful CI Main' >&2
  exit 1
fi

if ! grep -Fq 'image_name=${GITHUB_REPOSITORY,,}' "$backend_test_image_workflow"; then
  echo 'backend-test image name must use Bash lowercase normalization' >&2
  exit 1
fi

if ! grep -Fq 'backend-test-${{ github.event.workflow_run.head_sha }}' "$backend_test_image_workflow"; then
  echo 'backend-test image tag must use the CI Main workflow_run head SHA' >&2
  exit 1
fi

if grep -Fq 'backend-test-${{ github.sha }}' "$backend_test_image_workflow"; then
  echo 'backend-test image must not use the independent workflow SHA' >&2
  exit 1
fi

if [[ "$(grep -Fc -- '--partition hash:1/2' "$ci_main_workflow")" != 1 || "$(grep -Fc -- '--partition hash:2/2' "$ci_main_workflow")" != 1 ]]; then
  echo 'CI Main must run exactly one Stateful SQLite job for each hash partition' >&2
  exit 1
fi

if ! grep -Fq 'name: backend-test-archive-${{ github.run_id }}' "$ci_main_workflow"; then
  echo 'backend test archive must be scoped to the workflow run, not its attempt' >&2
  exit 1
fi

if grep -Fq 'name: backend-test-archive-${{ github.run_id }}-${{ github.run_attempt }}' "$ci_main_workflow"; then
  echo 'backend test archive must not include github.run_attempt' >&2
  exit 1
fi

if ! grep -Fq 'overwrite: true' "$ci_main_workflow"; then
  echo 'backend test archive producer must overwrite its run-scoped artifact on rerun' >&2
  exit 1
fi

python3 "$repo_root/.github/scripts/test-shared-testbox-api-read-smoke.py"

if ! grep -Fq -- '--entrypoint /bin/chmod' "$repo_root/scripts/shared-testbox-api-read-smoke"; then
  echo 'shared API smoke cleanup must make app-owned data removable before cleanup' >&2
  exit 1
fi

grep -q '^FROM production-runtime AS runtime$' "$dockerfile"

default_docker_stage="$(awk '/^FROM / { stage = $0 } END { print stage }' "$dockerfile")"
if [[ "$default_docker_stage" != 'FROM production-runtime AS runtime' ]]; then
  echo "default Docker build must produce the runtime image, got: $default_docker_stage" >&2
  exit 1
fi

release_amd_smoke_build="$(sed -n '/^      - name: Build smoke image (linux\/amd64, load)$/,/^      - name: Smoke test image (linux\/amd64)$/p' "$release_workflow")"
if ! grep -Fq 'target: runtime' <<<"$release_amd_smoke_build"; then
  echo 'release amd64 smoke build must use the runtime image target' >&2
  exit 1
fi

if ! grep -Fq -- '--target "runtime"' "$repo_root/.github/scripts/build-smoke-image-with-retry.sh"; then
  echo 'release arm64 smoke build helper must use the runtime image target' >&2
  exit 1
fi

set +e
missing_nextest_output="$(PATH=/usr/bin:/bin bash "$runner" --profile stateful-sqlite 2>&1)"
missing_nextest_rc=$?
set -e
[[ "$missing_nextest_rc" == 1 ]]
grep -q 'cargo-nextest is not installed' <<<"$missing_nextest_output"

printf '%s\n' 'test-backend-test-contract: all checks passed'
