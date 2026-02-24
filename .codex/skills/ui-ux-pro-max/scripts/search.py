#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
UI/UX Pro Max Search - BM25 search engine for UI/UX style guides
Usage: python search.py "<query>" [--domain <domain>] [--stack <stack>] [--max-results 3]
       python search.py "<query>" --design-system [-p "Project Name"]
       python search.py "<query>" --design-system --persist [-p "Project Name"] [--page "dashboard"]

Domains: style, color, chart, landing, product, ux, typography, icons, react, web
Stacks: html-tailwind, react, nextjs

Persistence (Master + Overrides pattern):
  --persist    Save design system to design-system/<project-slug>/MASTER.md
  --page       Also create a page-specific override file in design-system/<project-slug>/pages/
"""

import argparse
import sys
import io
from pathlib import Path
from core import CSV_CONFIG, AVAILABLE_STACKS, MAX_RESULTS, search, search_stack
from design_system import generate_design_system, _slugify_path_segment

def _ensure_utf8_stream(stream):
    """Best-effort UTF-8 output without assuming buffer-backed streams."""
    encoding = getattr(stream, "encoding", None)
    if not encoding or encoding.lower() == "utf-8":
        return stream

    # Prefer in-place reconfigure for text streams that support it.
    reconfigure = getattr(stream, "reconfigure", None)
    if callable(reconfigure):
        try:
            reconfigure(encoding="utf-8")
            return stream
        except Exception:
            pass

    # Fall back to wrapping the underlying buffer when available.
    buffer = getattr(stream, "buffer", None)
    if buffer is not None:
        return io.TextIOWrapper(buffer, encoding="utf-8")

    # Streams like StringIO have no buffer; leave them as-is.
    return stream


# Force UTF-8 for stdout/stderr to handle emojis on Windows (cp1252 default)
sys.stdout = _ensure_utf8_stream(sys.stdout)
sys.stderr = _ensure_utf8_stream(sys.stderr)


def format_output(result):
    """Format results for Claude consumption (token-optimized)"""
    if "error" in result:
        return f"Error: {result['error']}"

    output = []
    if result.get("stack"):
        output.append(f"## UI Pro Max Stack Guidelines")
        output.append(f"**Stack:** {result['stack']} | **Query:** {result['query']}")
    else:
        output.append(f"## UI Pro Max Search Results")
        output.append(f"**Domain:** {result['domain']} | **Query:** {result['query']}")
    output.append(f"**Source:** {result['file']} | **Found:** {result['count']} results\n")

    for i, row in enumerate(result['results'], 1):
        output.append(f"### Result {i}")
        for key, value in row.items():
            value_str = str(value)
            if len(value_str) > 300:
                value_str = value_str[:300] + "..."
            output.append(f"- **{key}:** {value_str}")
        output.append("")

    return "\n".join(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="UI Pro Max Search")
    parser.add_argument("query", help="Search query")
    mode_group = parser.add_mutually_exclusive_group()
    mode_group.add_argument("--domain", "-d", choices=list(CSV_CONFIG.keys()), help="Search domain")
    mode_group.add_argument("--stack", "-s", choices=AVAILABLE_STACKS, help="Stack-specific search (html-tailwind, react, nextjs)")
    parser.add_argument("--max-results", "-n", type=int, default=MAX_RESULTS, help="Max results (default: 3)")
    parser.add_argument("--json", action="store_true", help="Output as JSON")
    # Design system generation
    parser.add_argument("--design-system", "-ds", action="store_true", help="Generate complete design system recommendation")
    parser.add_argument("--project-name", "-p", type=str, default=None, help="Project name for design system output")
    parser.add_argument("--format", "-f", choices=["ascii", "markdown"], default="ascii", help="Output format for design system")
    # Persistence (Master + Overrides pattern)
    parser.add_argument("--persist", action="store_true", help="Save design system to design-system/<project-slug>/MASTER.md (creates hierarchical structure)")
    parser.add_argument("--page", type=str, default=None, help="Create page-specific override file in design-system/<project-slug>/pages/")
    parser.add_argument("--output-dir", "-o", type=str, default=None, help="Output directory for persisted files (default: current directory)")

    args = parser.parse_args()

    if args.page and not args.persist:
        parser.error("--page requires --persist")

    if (args.persist or args.page or args.output_dir) and not args.design_system:
        parser.error("--persist/--page/--output-dir can only be used with --design-system")

    if args.design_system and args.json:
        parser.error("--json is not supported with --design-system")

    # Design system takes priority
    if args.design_system:
        result = generate_design_system(
            args.query,
            args.project_name,
            args.format,
            persist=args.persist,
            page=args.page,
            output_dir=args.output_dir
        )
        print(result)

        # Print persistence confirmation
        if args.persist:
            project_name = args.project_name if args.project_name else args.query.upper()
            project_slug = _slugify_path_segment(project_name, "default")
            base_output_dir = Path(args.output_dir).resolve() if args.output_dir else Path.cwd()
            design_system_dir = base_output_dir / "design-system" / project_slug
            print("\n" + "=" * 60)
            print(f"✅ Design system persisted to {design_system_dir}/")
            print(f"   📄 {design_system_dir / 'MASTER.md'} (Global Source of Truth)")
            if args.page:
                page_filename = _slugify_path_segment(args.page, "page")
                print(f"   📄 {design_system_dir / 'pages' / f'{page_filename}.md'} (Page Overrides)")
            print("")
            print(f"📖 Usage: When building a page, check {design_system_dir / 'pages' / '[page].md'} first.")
            print(f"   If exists, its rules override MASTER.md. Otherwise, use MASTER.md.")
            print("=" * 60)
    # Stack search
    elif args.stack:
        result = search_stack(args.query, args.stack, args.max_results)
        if args.json:
            import json
            print(json.dumps(result, indent=2, ensure_ascii=False))
        else:
            print(format_output(result))
    # Domain search
    else:
        result = search(args.query, args.domain, args.max_results)
        if args.json:
            import json
            print(json.dumps(result, indent=2, ensure_ascii=False))
        else:
            print(format_output(result))
