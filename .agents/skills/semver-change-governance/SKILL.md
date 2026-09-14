---
name: semver-change-governance
description: "Apply the project's SemVer impact policy when a change may alter a public API or persistent state."
schema_version: 1
kind: policy-skill
slug: semver-change-governance
primary_topic: semver-change-governance
policy_dependencies: []
visibility: public
public_url_hosts: []
---

# SemVer change governance

Use this project policy when a release unit may change a public API or durable state. It records planned, current, and verified impact; it does not choose version numbers, labels, tags, migrations, or release tooling.

## Workflow

1. Define the release unit and record separate `public_api` and `persistent_state` impacts in `assets/templates/version-impact-record.example.json`.
2. Classify each contract as `patch`, `minor`, or `major` using compatibility evidence. Reclassify when scope or supported versions change.
3. Before release, verify every member and set the release unit to the highest verified impact.

## Compatibility rules

- `patch`: public API is nonbreaking; newly written state is readable by all supported earlier versions in the same Minor.
- `minor`: public API stays compatible within the Major; earlier Minor versions need not read newly migrated state, but the candidate must read supported earlier-Minor state.
- `major`: public API may break; state support is limited to the immediately preceding Major. Direct multi-Major jumps require explicit intermediate upgrades.

Keep API and state classifications separate. Migration mechanics belong to `persistent-state-migrations`; version identity and label projection belong to `pr-label-release`.

## Adoption preview

Before applying this policy to a target project, preview the writes to `.agents/skills/semver-change-governance/` and `skills-lock.json`, then wait for explicit owner approval. Install it with `npx skills add IvanLi-CN/style-playbook-skills --skill semver-change-governance --yes`; the CLI detects the Agent and project layout.

## Package resources

- `assets/templates/version-impact-record.example.json`
