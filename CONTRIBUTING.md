# Contributing to Lumbridge Speak

Lumbridge is at v0.1 and kept by a small team. Please read this first.

## Before you start

- **Bugs:** open an issue. Say what you did, what happened, what you expected, your OS, and the commit
  you built. For a word said wrong, give the text and what `speak text "<the text>"` prints.
- **Security problems:** do not open an issue. See [SECURITY.md](SECURITY.md).
- **Small fixes** (typos, clear bugs, a number read wrong): send a pull request.
- **Larger changes** (a new model adapter, new settings, a change to the HTTP API): open an issue first
  and wait for a reply.

## Build and test

You need Rust 1.85 or newer with `clippy`.

```sh
cargo build --locked
scripts/ci.sh
```

`scripts/ci.sh` is what CI runs: the scrub check,
`cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked`. The tests use a
stand-in model: no server and no sound. `cargo test -- --include-ignored` also checks the formats that
need `ffmpeg`.

The HTTP API answers byte for byte as documented in [docs/API.md](docs/API.md); `tests/api.rs` pins it.
If your change alters an answer, say so in the pull request.

## What we look for

- One change per pull request. Small is good.
- A test for each reading rule you add or change (`src/speakable.rs` has a table per kind of text).
- The README and `docs/` updated in the same pull request when behaviour changes.
- A line in `CHANGELOG.md` under "Unreleased" for anything a user would notice.
- Plain words in docs, comments and messages. Short sentences.
- No new network calls, telemetry or update checks.
- No new dependency without a reason in the pull request text.

## Never commit

- Audio of any kind (voice clips included), model weights, tokens or real `.env` files. `.gitignore`
  blocks most of them.
- Real names, addresses or phone numbers. Use made-up data: user `sam`, `example.com`, `555` numbers.
- Private host names, private network addresses or internal links. For addresses use the documentation
  ranges, such as `192.0.2.1`.

`scripts/scrub-check` looks for these and fails the build. If it flags something that is fine, add a
line to `.scrub-allow` with the reason and say so in the pull request.

## Third-party code

Say so in the pull request if any part of your change is copied or adapted from somewhere else, and give
the source and its licence. We cannot take code under a licence that conflicts with Apache-2.0.

## Pull requests

- Write the commit subject as a short sentence that says what changed.
- In the pull request, say what the change does, why, and how you tested it.
- A maintainer reviews and merges.

## Licence of your work

By sending a contribution you agree that it is licensed under Apache-2.0, the licence of this repository.

## Conduct

Be kind and direct. Argue about the work, not the person. Harassment and personal attacks get you removed.
