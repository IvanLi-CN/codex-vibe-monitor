#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

API_VERSION = "2022-11-28"


class ValidationError(RuntimeError):
    pass


@dataclass(frozen=True)
class RulesetRef:
    ruleset_id: int
    source_type: str | None
    source: str | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate the live GitHub branch rules against .github/quality-gates.json."
    )
    parser.add_argument(
        "--declaration",
        default=".github/quality-gates.json",
        help="Path to the quality gates declaration file.",
    )
    parser.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", ""),
        help="GitHub repository in owner/name form. Defaults to GITHUB_REPOSITORY.",
    )
    parser.add_argument(
        "--branch",
        default="",
        help="Protected branch to validate. Defaults to all protected branches declared in the contract.",
    )
    parser.add_argument(
        "--api-root",
        default=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
        help="GitHub API root URL. Defaults to GITHUB_API_URL or https://api.github.com.",
    )
    parser.add_argument(
        "--mode",
        choices=("auto", "require", "skip"),
        default=os.environ.get("QUALITY_GATES_LIVE_RULES_MODE", "auto"),
        help="skip: never validate; auto: validate only on GitHub Actions; require: always validate.",
    )
    parser.add_argument(
        "--rules-file",
        default="",
        help="Use a local branch-rules JSON fixture instead of calling the GitHub API.",
    )
    return parser.parse_args()


def should_skip(mode: str) -> bool:
    if mode == "skip":
        print("[live-quality-gates] skipped: QUALITY_GATES_LIVE_RULES_MODE=skip")
        return True
    if mode == "auto" and os.environ.get("GITHUB_ACTIONS") != "true":
        print("[live-quality-gates] skipped: outside GitHub Actions")
        return True
    return False


def load_declaration(path: str) -> dict[str, Any]:
    payload = json.loads(Path(path).read_text())
    if not isinstance(payload, dict):
        raise ValidationError("quality-gates declaration must be a JSON object")
    return payload


def choose_branches(declaration: dict[str, Any], override: str) -> list[str]:
    if override:
        return [override]
    raw_branches = (
        declaration.get("policy", {})
        .get("branch_protection", {})
        .get("protected_branches", [])
    )
    if not isinstance(raw_branches, list) or not raw_branches:
        raise ValidationError("protected_branches must declare at least one protected branch")
    branches: list[str] = []
    for index, branch in enumerate(raw_branches):
        if not isinstance(branch, str) or not branch:
            raise ValidationError(f"protected_branches[{index}] must be a non-empty string")
        if branch not in branches:
            branches.append(branch)
    return branches


def split_repo(repo: str) -> tuple[str, str]:
    owner, sep, name = repo.partition("/")
    if not sep or not owner or not name:
        raise ValidationError("--repo must be in owner/name form")
    return owner, name


def fetch_branch_rules(api_root: str, owner: str, repo: str, branch: str) -> Any:
    base_path = "/repos/{owner}/{repo}/rules/branches/{branch}".format(
        owner=urllib.parse.quote(owner, safe=""),
        repo=urllib.parse.quote(repo, safe=""),
        branch=urllib.parse.quote(branch, safe=""),
    )
    return fetch_paged_json(api_root, base_path)


def github_headers() -> dict[str, str]:
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "codex-vibe-monitor-quality-gates-live-check/1.0",
        "X-GitHub-Api-Version": API_VERSION,
    }
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def fetch_json(api_root: str, path: str) -> Any:
    url = api_root.rstrip("/") + path
    request = urllib.request.Request(url, headers=github_headers())
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise ValidationError(f"GitHub API request failed ({exc.code}): {detail}") from exc
    except urllib.error.URLError as exc:
        raise ValidationError(f"GitHub API request failed: {exc.reason}") from exc


def fetch_paged_json(api_root: str, base_path: str) -> list[Any]:
    rules: list[Any] = []
    page = 1
    while True:
        query = urllib.parse.urlencode({"per_page": 100, "page": page})
        payload = fetch_json(api_root, base_path + "?" + query)

        if isinstance(payload, dict) and isinstance(payload.get("data"), list):
            page_rules = payload["data"]
        elif isinstance(payload, list):
            page_rules = payload
        else:
            raise ValidationError("Unsupported GitHub branch rules payload type")

        rules.extend(page_rules)
        if len(page_rules) < 100:
            break
        page += 1

    return rules


def fetch_ruleset(api_root: str, owner: str, repo: str, ruleset_id: int) -> dict[str, Any]:
    path = "/repos/{owner}/{repo}/rulesets/{ruleset_id}".format(
        owner=urllib.parse.quote(owner, safe=""),
        repo=urllib.parse.quote(repo, safe=""),
        ruleset_id=ruleset_id,
    )
    payload = fetch_json(api_root, path)
    if not isinstance(payload, dict):
        raise ValidationError(f"Unsupported GitHub ruleset payload type for ruleset {ruleset_id}")
    return payload


def ruleset_ref_label(ref: RulesetRef) -> str:
    if ref.source:
        return f"ruleset:{ref.ruleset_id}@{ref.source}"
    return f"ruleset:{ref.ruleset_id}"


def placeholder_ruleset(ref: RulesetRef) -> dict[str, Any]:
    payload: dict[str, Any] = {"id": ref.ruleset_id}
    if ref.source_type:
        payload["source_type"] = ref.source_type
    if ref.source:
        payload["source"] = ref.source
    return payload


def merge_ruleset_ref(existing: RulesetRef, item: dict[str, Any]) -> RulesetRef:
    source_type = existing.source_type
    raw_source_type = item.get("ruleset_source_type") or item.get("source_type")
    if source_type is None and isinstance(raw_source_type, str) and raw_source_type:
        source_type = raw_source_type

    source = existing.source
    raw_source = item.get("ruleset_source") or item.get("source")
    if source is None and isinstance(raw_source, str) and raw_source:
        source = raw_source

    return RulesetRef(ruleset_id=existing.ruleset_id, source_type=source_type, source=source)


def extract_rules(payload: Any) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[RulesetRef]]:
    if isinstance(payload, dict) and isinstance(payload.get("data"), list):
        payload = payload["data"]
    if not isinstance(payload, list):
        raise ValidationError("Unsupported GitHub branch rules payload type")
    rules: list[dict[str, Any]] = []
    rulesets: list[dict[str, Any]] = []
    unresolved_rulesets: dict[int, RulesetRef] = {}
    for item in payload:
        if not isinstance(item, dict):
            continue
        nested_rules = item.get("rules")
        if isinstance(nested_rules, list):
            rulesets.append(item)
            rules.extend(
                rule for rule in nested_rules if isinstance(rule, dict) and isinstance(rule.get("type"), str)
            )
            continue
        if isinstance(item.get("type"), str):
            rules.append(item)
        raw_ruleset_id = item.get("ruleset_id")
        if isinstance(raw_ruleset_id, int):
            current = unresolved_rulesets.get(raw_ruleset_id) or RulesetRef(
                ruleset_id=raw_ruleset_id,
                source_type=None,
                source=None,
            )
            unresolved_rulesets[raw_ruleset_id] = merge_ruleset_ref(current, item)
    if not rules:
        raise ValidationError("GitHub branch rules payload did not contain any typed rules")
    return rules, rulesets, sorted(unresolved_rulesets.values(), key=lambda ref: ref.ruleset_id)


def ruleset_label(ruleset: dict[str, Any]) -> str:
    name = ruleset.get("name")
    if isinstance(name, str) and name:
        return name
    ruleset_id = ruleset.get("id")
    if isinstance(ruleset_id, int):
        source = ruleset.get("source")
        if isinstance(source, str) and source:
            return f"ruleset:{ruleset_id}@{source}"
        return f"ruleset:{ruleset_id}"
    source = ruleset.get("source")
    if isinstance(source, str) and source:
        return source
    return "ruleset:unknown"


def describe_bypass_actor(actor: dict[str, Any]) -> str:
    actor_type = actor.get("actor_type")
    if not isinstance(actor_type, str) or not actor_type:
        actor_type = "unknown"
    actor_id = actor.get("actor_id")
    bypass_mode = actor.get("bypass_mode")
    parts = [actor_type]
    if actor_id is not None:
        parts.append(str(actor_id))
    if isinstance(bypass_mode, str) and bypass_mode:
        parts.append(bypass_mode)
    return "/".join(parts)


def validate_bypass_actors(
    declaration: dict[str, Any],
    rulesets: list[dict[str, Any]],
    branch: str,
) -> tuple[list[str], list[str], str]:
    notes: list[str] = []
    errors: list[str] = []
    bypass_reason = branch_waivers(declaration, branch)
    if not rulesets:
        if bypass_reason:
            notes.append(f"{branch}: bypass actor verification waived explicitly ({bypass_reason})")
        else:
            notes.append(f"{branch}: bypass actor verification unavailable (status=unverified)")
        return errors, notes, "unverified"

    unavailable_rulesets: list[str] = []
    violating_rulesets: list[str] = []
    for ruleset in rulesets:
        actors = ruleset.get("bypass_actors")
        if not isinstance(actors, list):
            unavailable_rulesets.append(ruleset_label(ruleset))
            continue
        if not actors:
            continue
        violating_rulesets.append(
            f"{ruleset_label(ruleset)}[{', '.join(describe_bypass_actor(actor) for actor in actors if isinstance(actor, dict))}]"
        )

    if violating_rulesets:
        errors.append(f"{branch}: bypass actors must stay empty ({'; '.join(violating_rulesets)})")
    if unavailable_rulesets:
        if bypass_reason:
            notes.append(
                f"{branch}: bypass actor verification waived explicitly for {', '.join(unavailable_rulesets)} ({bypass_reason}; status=unverified)"
            )
        else:
            notes.append(
                f"{branch}: bypass actor verification unavailable for {', '.join(unavailable_rulesets)} (status=unverified)"
            )
        return errors, notes, "unverified"
    return errors, notes, "verified"


def bool_field(parameters: dict[str, Any], name: str) -> bool:
    return bool(parameters.get(name, False))


def normalize_required_status_checks(
    rules: list[dict[str, Any]],
) -> tuple[list[str], dict[str, set[int | None]], set[bool]]:
    contexts: set[str] = set()
    integrations: dict[str, set[int | None]] = {}
    strict_values: set[bool] = set()
    for rule in rules:
        parameters = rule.get("parameters") or {}
        if not isinstance(parameters, dict):
            continue
        strict = parameters.get("strict_required_status_checks_policy")
        if isinstance(strict, bool):
            strict_values.add(strict)
        raw_checks = parameters.get("required_status_checks") or []
        if not isinstance(raw_checks, list):
            continue
        for item in raw_checks:
            if not isinstance(item, dict):
                continue
            context = item.get("context")
            if isinstance(context, str) and context:
                contexts.add(context)
                integration = item.get("integration_id")
                if integration is not None and not isinstance(integration, int):
                    integration = None
                integrations.setdefault(context, set()).add(integration)
    return sorted(contexts), integrations, strict_values


def branch_waivers(declaration: dict[str, Any], branch: str) -> str | None:
    raw_waivers = declaration.get("waivers", [])
    if raw_waivers is None:
        raw_waivers = []
    if not isinstance(raw_waivers, list):
        raise ValidationError("waivers must be a JSON array")

    bypass_reason: str | None = None
    for index, waiver in enumerate(raw_waivers):
        if not isinstance(waiver, dict):
            raise ValidationError(f"waivers[{index}] must be a JSON object")
        kind = waiver.get("kind")
        waiver_branch = waiver.get("branch")
        reason = waiver.get("reason")
        if not isinstance(kind, str) or not kind:
            raise ValidationError(f"waivers[{index}].kind must be a non-empty string")
        if not isinstance(waiver_branch, str) or not waiver_branch:
            raise ValidationError(f"waivers[{index}].branch must be a non-empty string")
        if not isinstance(reason, str) or not reason:
            raise ValidationError(f"waivers[{index}].reason must be a non-empty string")
        if waiver_branch != branch:
            continue
        if kind == "bypass-actors-unverified":
            bypass_reason = reason
            continue
        raise ValidationError(
            f"waivers[{index}].kind={kind!r} is unsupported; only bypass-actors-unverified is allowed"
        )

    return bypass_reason


def hydrate_branch_rules(
    rules_fixture: Any,
    api_root: str,
    owner: str,
    repo: str,
    branch: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[str]]:
    payload = rules_fixture if rules_fixture is not None else fetch_branch_rules(api_root, owner, repo, branch)
    rules, rulesets, unresolved_rulesets = extract_rules(payload)
    hydrated_rulesets = list(rulesets)
    known_ruleset_ids = {
        item["id"] for item in hydrated_rulesets if isinstance(item.get("id"), int)
    }
    notes: list[str] = []
    for ref in unresolved_rulesets:
        if ref.ruleset_id in known_ruleset_ids:
            continue
        if rules_fixture is None:
            ruleset_payload = fetch_ruleset(api_root, owner, repo, ref.ruleset_id)
            if ruleset_payload.get("id") != ref.ruleset_id:
                raise ValidationError(f"GitHub ruleset payload drifted for {ruleset_ref_label(ref)}")
            if ref.source and not ruleset_payload.get("source"):
                ruleset_payload["source"] = ref.source
            if ref.source_type and not ruleset_payload.get("source_type"):
                ruleset_payload["source_type"] = ref.source_type
            hydrated_rulesets.append(ruleset_payload)
        else:
            hydrated_rulesets.append(placeholder_ruleset(ref))
            notes.append(
                f"{branch}: ruleset details unavailable for {ruleset_ref_label(ref)} when using a local fixture"
            )
        known_ruleset_ids.add(ref.ruleset_id)
    return rules, hydrated_rulesets, notes


def main() -> int:
    args = parse_args()
    if should_skip(args.mode):
        return 0

    try:
        declaration = load_declaration(args.declaration)
        branches = choose_branches(declaration, args.branch)
        owner, repo = split_repo(args.repo)
        rules_fixture = json.loads(Path(args.rules_file).read_text()) if args.rules_file else None
        errors: list[str] = []
        notes: list[str] = []
        checked_rules: dict[str, list[str]] = {}
        bypass_actor_status: dict[str, str] = {}
        for branch in branches:
            rules, hydrated_rulesets, branch_notes = hydrate_branch_rules(
                rules_fixture, args.api_root, owner, repo, branch
            )
            notes.extend(branch_notes)
            checked_rules[branch] = sorted({rule.get("type", "") for rule in rules})
            branch_errors, branch_notes, branch_bypass_status = validate_rules(
                declaration,
                rules,
                hydrated_rulesets,
                branch,
            )
            errors.extend(branch_errors)
            notes.extend(branch_notes)
            bypass_actor_status[branch] = branch_bypass_status
    except ValidationError as exc:
        print(f"[live-quality-gates] {exc}", file=sys.stderr)
        return 1

    if errors:
        print("[live-quality-gates] drift detected:", file=sys.stderr)
        for item in errors:
            print(f"- {item}", file=sys.stderr)
        return 1

    print(
        json.dumps(
            {
                "status": "ok",
                "repo": args.repo,
                "branches": branches,
                "checked_rules": checked_rules,
                "bypass_actor_status": bypass_actor_status,
                "notes": [
                    "Validated effective branch rules; bypass actor status is reported per branch and remains unverified unless ruleset payloads explicitly expose bypass_actors.",
                    *notes,
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
