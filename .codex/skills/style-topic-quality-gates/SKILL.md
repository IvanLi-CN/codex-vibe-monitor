---
name: style-topic-quality-gates
description: "Topic skill for 'quality gates' generated from style playbook tags."
---

# Topic: quality gates

This topic skill is generated from `skills/style-playbook/references/tags/quality-gates.md`.

## Starter Required

Load `$style-playbook-starter` first for the standard maintenance workflow:

- patch (write)
- enhance (write)
- update (read)

## Source

- Tag token: `quality gates`
- Tag doc: `skills/style-playbook/references/tags/quality-gates.md`

## Usage

- Use this skill when you need guidance scoped to repo-level PR quality gates, required-check declarations, and required-check drift handling.
- Treat this topic as a two-layer contract: a fixed template shared across repos plus dynamic adjustment rules based on each repo's actual PR workflows and check names; the fixed layer includes required-check declaration, conditional review baseline, signed-commit enforcement, and default-branch protection.
- Start from `templates/quality-gates.example.json`, `templates/review-policy.yml`, and `templates/waiver-record.example.json`, then use `scripts/check_quality_gates.py` to validate the filled contract.
- GitHub alignment must preserve the declaration semantics. For actor-conditional review policy, prefer a dedicated required policy-check workflow; use split native rulesets only as an explicit fallback when the repo accepts bypass UI noise, and do not silently flatten the contract into blanket approvals.
- Treat this topic as the contract for initializing or syncing those rules inside a repository; it does not silently mutate GitHub rulesets or branch protection by itself.
- Keep the durable policy in the tag doc and repo-local declarations; keep maintenance workflow in `$style-playbook-starter`.
