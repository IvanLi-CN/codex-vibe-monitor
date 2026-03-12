#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

extract_script() {
  local workflow_path="$1"
  local job_name="$2"
  local output_path="$3"
  ruby - <<'RUBY' "$workflow_path" "$job_name" "$output_path"
require "yaml"

workflow_path, job_name, output_path = ARGV
workflow = YAML.load_file(workflow_path)
job = workflow.fetch("jobs").fetch(job_name)
step = job.fetch("steps").find { |item| item["uses"] == "actions/github-script@v8" }
abort("missing github-script step in #{workflow_path}:#{job_name}") unless step
script = step.dig("with", "script")
abort("missing github-script body in #{workflow_path}:#{job_name}") unless script.is_a?(String) && !script.empty?
File.write(output_path, script)
RUBY
}

extract_script "$repo_root/.github/workflows/label-gate.yml" "label-gate" "$tmp_dir/label-gate.js"
extract_script "$repo_root/.github/workflows/review-policy.yml" "review-policy" "$tmp_dir/review-policy.js"

node - <<'NODE' "$tmp_dir/label-gate.js" "$tmp_dir/review-policy.js"
const fs = require('fs');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

const [labelPath, reviewPath] = process.argv.slice(2);

async function runWorkflowScript(scriptPath, { context, github, env }) {
  const script = fs.readFileSync(scriptPath, 'utf8');
  const logs = [];
  let failure = null;
  const core = {
    info(message) {
      logs.push(String(message));
    },
    setFailed(message) {
      failure = String(message);
    },
    summary: {
      addHeading() {
        return this;
      },
      addRaw() {
        return this;
      },
      async write() {},
    },
  };
  const proc = {
    env: {
      ...process.env,
      ...env,
    },
  };
  const fn = new AsyncFunction('context', 'github', 'core', 'process', script);
  let thrown = null;
  try {
    await fn(context, github, core, proc);
  } catch (error) {
    thrown = error;
  }
  return { logs, failure, thrown };
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function testLabelGateMergeGroupAnchors() {
  const github = {
    rest: {
      issues: {
        get: async ({ issue_number }) => ({
          data: {
            labels: issue_number === 42
              ? [{ name: 'type:patch' }, { name: 'channel:stable' }]
              : [{ name: 'type:minor' }, { name: 'channel:rc' }],
          },
        }),
      },
    },
  };
  const context = {
    eventName: 'merge_group',
    ref: 'refs/heads/gh-readonly-queue/main/pr-42-a1b2c3d4/pr-57-ffeeddcc',
    payload: {
      merge_group: {
        base_ref: 'refs/heads/main',
      },
    },
    repo: {
      owner: 'IvanLi-CN',
      repo: 'codex-vibe-monitor',
    },
  };
  const result = await runWorkflowScript(labelPath, {
    context,
    github,
    env: {
      GITHUB_REF: 'refs/heads/gh-readonly-queue/main/pr-42-a1b2c3d4/pr-57-ffeeddcc',
      MANUAL_PULL_NUMBER: '',
    },
  });
  assert(!result.thrown, `label-gate threw unexpectedly: ${result.thrown && result.thrown.message}`);
  assert(!result.failure, `label-gate failed unexpectedly: ${result.failure}`);
  assert(
    result.logs.some((entry) => entry.includes('label gate validated 2 pull request(s)')),
    'label-gate did not validate all merge-group ref anchors',
  );
}

async function testReviewPolicyMergeGroupAnchors() {
  const permissions = {
    alice: 'write',
    bob: 'write',
    reviewer: 'write',
  };
  const authors = {
    42: 'alice',
    57: 'bob',
  };
  const github = {
    paginate: async (route, params) => {
      if (route === 'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews') {
        return [
          {
            user: { login: 'reviewer' },
            state: 'APPROVED',
            submitted_at: '2026-03-12T00:00:00Z',
          },
        ];
      }
      throw new Error(`unexpected paginate route: ${route} ${JSON.stringify(params || {})}`);
    },
    request: async (route, params) => {
      if (route !== 'GET /repos/{owner}/{repo}/collaborators/{username}/permission') {
        throw new Error(`unexpected request route: ${route}`);
      }
      return {
        data: {
          permission: permissions[String(params.username)] || 'none',
        },
      };
    },
    rest: {
      pulls: {
        get: async ({ pull_number }) => ({
          data: {
            user: {
              login: authors[pull_number],
            },
          },
        }),
      },
    },
  };
  const context = {
    eventName: 'merge_group',
    ref: 'refs/heads/gh-readonly-queue/main/pr-42-a1b2c3d4/pr-57-ffeeddcc',
    payload: {
      merge_group: {
        base_ref: 'refs/heads/main',
      },
    },
    repo: {
      owner: 'IvanLi-CN',
      repo: 'codex-vibe-monitor',
    },
  };
  const result = await runWorkflowScript(reviewPath, {
    context,
    github,
    env: {
      GITHUB_REF: 'refs/heads/gh-readonly-queue/main/pr-42-a1b2c3d4/pr-57-ffeeddcc',
      MANUAL_PULL_NUMBER: '',
    },
  });
  assert(!result.thrown, `review-policy threw unexpectedly: ${result.thrown && result.thrown.message}`);
  assert(!result.failure, `review-policy failed unexpectedly: ${result.failure}`);
  assert(
    result.logs.some((entry) => entry.includes('review gate validated 2 pull request(s)')),
    'review-policy did not validate all merge-group ref anchors',
  );
}

async function testMergeGroupWithoutAnchorsFailsClosed() {
  const context = {
    eventName: 'merge_group',
    ref: 'refs/heads/gh-readonly-queue/main/group-opaque',
    payload: {
      merge_group: {
        base_ref: 'refs/heads/main',
      },
    },
    repo: {
      owner: 'IvanLi-CN',
      repo: 'codex-vibe-monitor',
    },
  };
  const result = await runWorkflowScript(labelPath, {
    context,
    github: {
      rest: {
        issues: {
          get: async () => {
            throw new Error('label lookup should not run without merge-group anchors');
          },
        },
      },
    },
    env: {
      GITHUB_REF: 'refs/heads/gh-readonly-queue/main/group-opaque',
      MANUAL_PULL_NUMBER: '',
    },
  });
  assert(result.thrown, 'label-gate should fail closed when merge-group anchors are missing');
  assert(
    String(result.thrown && result.thrown.message).includes('could not be proven'),
    `label-gate returned unexpected anchor failure: ${result.thrown && result.thrown.message}`,
  );
}

Promise.resolve()
  .then(testLabelGateMergeGroupAnchors)
  .then(testReviewPolicyMergeGroupAnchors)
  .then(testMergeGroupWithoutAnchorsFailsClosed)
  .catch((error) => {
    console.error(error.message || error);
    process.exit(1);
  });
NODE

echo "test-inline-metadata-workflows: all checks passed"
