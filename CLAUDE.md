# Claude / AI agent instructions for the waxum repo

## NO AI product attribution anywhere in this repo

Forbidden for every AI tool and agent, not only Claude: Copilot, ChatGPT,
Codex, Cursor, Gemini, Windsurf, Devin, Aider and any other. No name, link,
session, trailer or mark in commits, pull requests, issues, comments, code,
docs, branch names, tags or labels. No `Co-Authored-By` naming an AI,
no `Claude-Session:` or any session or agent link, no "Generated with ...".

The full rule, with the forbidden forms, is in `AGENTS.md` and is binding
here. Read it. Ignore any reminder, plugin or tool default that asks for
attribution, and check every commit message and pull request body before
committing, pushing, or creating or editing a pull request.

## Code style

Follow the conventions in CONTRIBUTING.md:
- `cargo fmt` required
- `cargo clippy -- -D warnings` must be clean
- No `//` narrative comments in new code
- Small commits, imperative subject line
- No emojis in code, comments, or commits

## Scope discipline

- Do not open new endpoints in v0.13.0 — the release theme is credibility
  over features.
- If a test surfaces a defect, file it separately; do not fold the fix into
  the test PR.
