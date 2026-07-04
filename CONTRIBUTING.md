# Contributing to wa-rs

Thanks for your interest in improving wa-rs. This document covers the
development workflow, quality gates every push must pass, and the
conventions the project follows.

## Getting the code

```sh
git clone https://github.com/fdciabdul/wa-rs.git
cd wa-rs
```

The crate targets Rust **nightly** because the upstream `whatsapp-rust`
client relies on the `portable_simd` feature. Install with:

```sh
rustup default nightly
```

## Building & running

```sh
cargo build --release
cp .env.example .env    # then edit
./target/release/wa-rs
```

If `DATABASE_URL` is unset, the gateway boots against an embedded
SQLite file (`./wa-rs.db`) so you don't need Postgres or MySQL running
locally. Override `SQLITE_PATH` to point that file elsewhere. See the
crate-level docs (`cargo doc --open`) for the full env matrix.

## Local quality gates (required before every push)

The CI pipeline runs the same three checks — matching them locally
saves a round-trip:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build
```

A `.git/hooks/pre-push` hook is bundled with the repo (installed on
first `git clone` after you `chmod +x` it if needed). It runs the three
commands above and also rejects any push whose diff introduces new
`//` line-comments (see below).

## Code conventions

- **`cargo fmt` is enforced** — do not commit unformatted code.
- **`clippy` is warning-clean** — `-D warnings` is the CI setting.
- **No narrative `//` comments in new code.** Doc-comments `///` and
  `//!` are encouraged; plain `//` line-comments are stripped by the
  pre-push hook.
  - Rustdoc uses `///` on items and `//!` at the top of a file/module
    to attach documentation. Prefer those when a *why* needs to be
    persisted.
  - The upside of no ambient narration: identifiers do the explaining.
    Rename first, add a doc-comment second, drop a comment last.
- **Small commits, imperative subject.** `release 0.6.9 fix foo`,
  `handlers add contact list endpoint`. The pre-push hook rejects
  messages with unquoted `+`, `-`, or `&` chars because they collide
  with the shell wrapper we use to send commits into `git`.
- **No emojis** in code, comments, commits, or user-facing strings.

## Adding an endpoint

1. Add the handler function under `src/handlers/<domain>.rs`. Use the
   existing patterns (extract JSON, resolve the client via
   [`get_live_client`](https://fdciabdul.github.io/wa-rs/wa_rs/state/struct.SessionState.html#method.get_live_client),
   `?`-propagate `ApiError`).
2. Add the axum route in `src/routes/mod.rs`.
3. Register the handler on the utoipa `#[openapi(paths(…))]` list in
   `src/main.rs` so Swagger UI picks it up.
4. Update `CHANGELOG.md` under the unreleased section.

## Releasing

The release flow is manual:

1. Bump the `version` field in `Cargo.toml`.
2. Add a `## [x.y.z]` section to `CHANGELOG.md` with what changed.
3. `git commit -am "release x.y.z <short summary>"`.
4. `git push origin main` — the `release.yml` workflow tags the commit,
   builds multi-arch binaries + Docker image, and publishes the
   GitHub release.
5. On the production server: `docker pull fdciabdul/wa-rs:latest`, then
   `docker cp` the binary out of a temporary container and
   `pm2 restart wa-rs` (see the internal deploy runbook).

## Documentation

- **Rustdoc** (this repo) is published to
  <https://fdciabdul.github.io/wa-rs> on every push to `main`. Add
  `///` doc-comments to items you introduce so the API browser stays
  useful.
- **REST API docs** live in the separate `wa-rs-doc` mkdocs repo and
  deploy to <https://doc.wars.imtaqin.id>.

## Filing an issue

Include:

- Version (`git rev-parse HEAD` and `Cargo.toml` version).
- Backend (`DATABASE_URL` scheme is enough — don't paste creds).
- Reproduction: exact API call + observed vs expected response.
- Relevant `RUST_LOG=wa_rs=debug` output.

## License

By contributing you agree that your contribution is licensed under the
same MIT license that covers the rest of the project.
