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

async function testLabelGatePullRequestEvent() {
  const github = {
    rest: {
      issues: {
        get: async ({ issue_number }) => ({
          data: {
            labels: issue_number === 57
              ? [{ name: 'type:minor' }, { name: 'channel:rc' }]
              : [{ name: 'type:patch' }, { name: 'channel:stable' }],
          },
        }),
      },
    },
  };
  const context = {
    eventName: 'pull_request',
    payload: {
      pull_request: {
        number: 57,
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
      MANUAL_PULL_NUMBER: '',
    },
  });
  assert(!result.thrown, `label-gate threw unexpectedly: ${result.thrown && result.thrown.message}`);
  assert(!result.failure, `label-gate failed unexpectedly: ${result.failure}`);
  assert(
    result.logs.some((entry) => entry.includes('label gate validated 1 pull request(s)')),
    'label-gate did not validate the pull_request payload',
  );
}

async function testLabelGateFailureCases() {
  const github = {
    rest: {
      issues: {
        get: async ({ issue_number }) => ({
          data: {
            labels: issue_number === 58
              ? [{ name: 'type:patch' }, { name: 'type:minor' }, { name: 'channel:stable' }]
              : [{ name: 'type:patch' }],
          },
        }),
      },
    },
  };

  for (const pullNumber of [58, 59]) {
    const result = await runWorkflowScript(labelPath, {
      context: {
        eventName: 'pull_request',
        payload: {
          pull_request: {
            number: pullNumber,
          },
        },
        repo: {
          owner: 'IvanLi-CN',
          repo: 'codex-vibe-monitor',
        },
      },
      github,
      env: {},
    });
    assert(!result.thrown, `label-gate failure case threw unexpectedly: ${result.thrown && result.thrown.message}`);
    assert(result.failure, `label-gate should fail for PR #${pullNumber}`);
  }
}

async function testReviewPolicyReviewEvent() {
  const permissions = {
    bob: 'write',
    reviewer: 'write',
  };
  const authors = {
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
            head: {
              sha: `sha-${pull_number}`,
            },
          },
        }),
      },
      repos: {
        createCommitStatus: async () => ({}),
      },
    },
  };
  const context = {
    eventName: 'pull_request_review',
    payload: {
      pull_request: {
        number: 57,
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
    env: {},
  });
  assert(!result.thrown, `review-policy threw unexpectedly: ${result.thrown && result.thrown.message}`);
  assert(!result.failure, `review-policy failed unexpectedly: ${result.failure}`);
  assert(
    result.logs.some((entry) => entry.includes('review gate validated 1 pull request(s)')),
    'review-policy did not validate the pull_request_review payload',
  );
}

async function testReviewPolicyFailureCases() {
  const permissions = {
    bob: 'write',
    reviewer: 'write',
  };
  const reviewMatrix = {
    58: [],
    59: [
      {
        user: { login: 'reviewer' },
        state: 'APPROVED',
        submitted_at: '2026-03-12T00:00:00Z',
      },
      {
        user: { login: 'reviewer' },
        state: 'CHANGES_REQUESTED',
        submitted_at: '2026-03-12T00:05:00Z',
      },
    ],
    60: [
      {
        user: { login: 'reviewer' },
        state: 'APPROVED',
        submitted_at: '2026-03-12T00:00:00Z',
      },
      {
        user: { login: 'reviewer' },
        state: 'DISMISSED',
        submitted_at: '2026-03-12T00:05:00Z',
      },
    ],
  };
  const github = {
    paginate: async (route, params) => {
      if (route === 'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews') {
        return reviewMatrix[params.pull_number] || [];
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
              login: 'bob',
            },
            head: {
              sha: `sha-${pull_number}`,
            },
          },
        }),
      },
      repos: {
        createCommitStatus: async () => ({}),
      },
    },
  };

  for (const pullNumber of [58, 59, 60]) {
    const result = await runWorkflowScript(reviewPath, {
      context: {
        eventName: 'pull_request_review',
        payload: {
          pull_request: {
            number: pullNumber,
          },
        },
        repo: {
          owner: 'IvanLi-CN',
          repo: 'codex-vibe-monitor',
        },
      },
      github,
      env: {},
    });
    assert(!result.thrown, `review-policy failure case threw unexpectedly: ${result.thrown && result.thrown.message}`);
    assert(result.failure, `review-policy should fail for PR #${pullNumber}`);
  }
}

Promise.resolve()
  .then(testLabelGatePullRequestEvent)
  .then(testLabelGateFailureCases)
  .then(testReviewPolicyReviewEvent)
  .then(testReviewPolicyFailureCases)
  .catch((error) => {
    console.error(error.message || error);
    process.exit(1);
  });
NODE

echo "test-inline-metadata-workflows: all checks passed"
