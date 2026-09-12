# Claude / AI agent instructions for the waxum repo

## Commit messages

Do NOT add any of the following to commit messages:

- `Co-Authored-By: Claude …`
- `Co-Authored-By: Claude Opus …` / `Co-Authored-By: Claude Sonnet …`
- `Generated with Claude Code`
- `🤖 Generated with …`
- Any line containing `noreply@anthropic.com`

These trailers are stripped by the `commit-msg` hook and are unwanted in
this repo's history. Omit them entirely; do not add them expecting the
hook to clean up.

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
