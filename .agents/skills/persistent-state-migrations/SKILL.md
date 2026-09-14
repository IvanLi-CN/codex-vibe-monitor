---
name: persistent-state-migrations
description: "Design and verify release-to-release migrations for durable application, file, queue, or device state."
schema_version: 1
kind: policy-skill
slug: persistent-state-migrations
primary_topic: persistent-state-migrations
policy_dependencies: []
visibility: public
public_url_hosts: []
---

# Persistent-state migrations

Use this project policy whenever a release creates, reads, transforms, replaces, or stops reading state that survives the running process. This includes database schemas, files, queues, device configuration, and embedded storage. Temporary caches and non-persistent fixtures are out of scope.

## Workflow

1. Record the state, source compatibility range, ordered operations, rollout signals, recovery path, and validation in `assets/templates/persistent-state-migration-record.example.json`.
2. Keep structural changes, DML, and historical-data backfills separate, ordered, observable, and pauseable.
3. Treat deployed migrations as immutable. A stopped release is repaired forward; program rollback does not silently reverse persisted state. Backup restore is separate disaster recovery.
4. Test every declared earlier-version source state and the stopped-release repair path. Verify idempotent recognition and safe re-entry.

When public APIs also change, read the installed `semver-change-governance` policy and record the two compatibility contracts independently. This policy does not select tags, labels, or version identity.

## Adoption preview

Before applying this policy to a target project, preview the writes to `.agents/skills/persistent-state-migrations/` and `skills-lock.json`, then wait for explicit owner approval. Install it with `npx skills add IvanLi-CN/style-playbook-skills --skill persistent-state-migrations --yes`; the CLI detects the Agent and project layout.

## Package resources

- `assets/templates/persistent-state-migration-record.example.json`
