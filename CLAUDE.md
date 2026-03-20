# CLAUDE.md

## gstack

For all web browsing, use the `/browse` skill from gstack. Never use `mcp__claude-in-chrome__*` tools directly.

### Available Skills

- `/office-hours` — Brainstorm and validate ideas (YC-style)
- `/plan-ceo-review` — CEO/founder strategy review
- `/plan-eng-review` — Engineering architecture review
- `/plan-design-review` — Design plan review
- `/design-consultation` — Create a design system / DESIGN.md
- `/review` — Pre-landing PR code review
- `/ship` — Ship workflow (test, review, bump, PR)
- `/browse` — Fast headless browser for QA and dogfooding
- `/qa` — QA test and fix bugs
- `/qa-only` — QA report only (no fixes)
- `/design-review` — Visual design audit and polish
- `/setup-browser-cookies` — Import cookies for authenticated testing
- `/retro` — Weekly engineering retrospective
- `/investigate` — Systematic debugging with root cause analysis
- `/document-release` — Post-ship documentation update
- `/codex` — Second opinion via OpenAI Codex CLI
- `/careful` — Safety guardrails for destructive commands
- `/freeze` — Restrict edits to a specific directory
- `/guard` — Full safety mode (careful + freeze)
- `/unfreeze` — Remove edit restrictions
- `/gstack-upgrade` — Upgrade gstack to latest version

## Design System
Always read DESIGN.md before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.
