# Contributing

## Boundaries

This is a read-only monitor, not an OBS control surface. Preserve the downward
`obs → metrics → tui` dependencies, total decoders, per-stream baselines,
redacted passwords, and stable `once` JSON shape. UI state transitions and
widgets must not perform I/O. See [CLAUDE.md](CLAUDE.md) for the architectural
invariants and [docs/design.md](docs/design.md) for telemetry/preview contracts.

Reuse existing dependencies and semantic theme tokens. Every new glyph must
have an ASCII fallback; aligned icons must occupy one terminal column. Every
layout change must update its current-frame mouse regions and test actual drawn
coordinates, not hand-maintained rectangles.

## Local quality gates

Rust 1.88 or newer is required. Check the committed lockfile, not a fresh
resolution of dependencies:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo test --locked --example screenshot
cargo build --locked
python3 scripts/pty_smoke.py --binary target/debug/obs-stats
```

The Python smoke test uses a real POSIX pseudoterminal and an isolated missing
config/closed port. It needs no OBS instance, credentials, or third-party Python
packages. Run it on Linux; platform rendering tests also run on Windows/macOS.
Keep actual OBS checks separate from synthetic transport/model fixtures.

## Visual changes

Documentation images are actual Ratatui cell buffers, not mockups. Regenerate
with synthetic data, inspect the text layout and raw backgrounds, then confirm
a second generation leaves the images byte-identical:

```sh
cargo run --locked --example screenshot -- --demo
cargo run --locked --example screenshot -- --view overview --size 80x24 --scenario healthy --text
cargo run --locked --example screenshot -- --scenario stale --cells --output "$TMPDIR/obs-stats-cells"
cargo run --locked --example screenshot -- --matrix --output "$TMPDIR/obs-stats-matrix"
```

Custom previews default to ignored `docs/previews`; the ordinary `--demo`
entry point writes the tracked `docs/images` set. Never commit live OBS captures
or credentials as fixture data. Relative sample age is rendered through
`ui::draw_at`, so captures freeze presentation time as well as acquisition time.

## Releases

1. Review the intended diff and keep unrelated changes out of the index.
2. Update `Cargo.toml`, its package entry in `Cargo.lock`, and `CHANGELOG.md`.
3. Pass local gates and deterministic screenshot reproduction.
4. Commit and push the intended changes; wait for CI on that exact commit.
5. Create an annotated `v<package-version>` tag on the verified commit and push it.
6. The release workflow validates the tag/version, runs quality gates, builds
   the platform matrix, verifies packages/checksums, and publishes the assets.
7. Read back the remote tag, release state and assets; download and verify the
   native archive before reporting the release complete.

Do not move a published tag to fix a release. Fix forward with a new version.
