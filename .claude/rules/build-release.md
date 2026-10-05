## Build

```plaintext
make build     # cargo build --release --workspace   (all three crates)
make install   # cargo install --path crates/ss-magic
make test      # cargo test --workspace --locked
make clean     # cargo clean
```

Rust toolchain is provided by `rustup` (cargo on `~/.cargo/bin`).

The repository is a Cargo **workspace with three members**: the root
`Cargo.toml` is a virtual manifest (no `[package]`) owning `[workspace.package]`
(edition, repository, license) and BOTH profiles – `[profile.release]` with
`opt-level = "z"` (a measured decision, KTD9 of the workspace-split plan:
`opt-level = 3` ran about 0.13 ms faster for a hook binary 47 percent larger; do
not tune it per crate) and `[profile.dist]` inheriting it. Members live under
`crates/`:

- `crates/ss-magic-core` – the shared library. `publish = false` plus
  `[package.metadata.dist] dist = false`, version `0.1.0`, never a release
  surface and never tagged.
- `crates/ss-magic` – binary `ss-magic`, the sync CLI. Currently `0.11.2`.
- `crates/ss-magic-plugin` – binary `ss-magic-plugin`, the plugin's hook runtime
  and verb tree. Currently `1.0.1`.

Every `cargo` command is run from the root with `--workspace`; `cargo install
--path` needs a `[package]`, so it names `crates/ss-magic`. `make install`
installs the CLI ALONE, deliberately: the plugin binary is delivered by the
marketplace into `${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin`, and nothing looks
for it on `PATH`, so a development copy there would be picked up by nothing.

### Two release lines

`ss-magic` releases on bare `vX.Y.Z` tags; `ss-magic-plugin` releases on
`ss-magic-plugin-vX.Y.Z`. **Their versions must always DIFFER.** cargo-dist
parses a tag as `[PACKAGE_NAME-]VERSION`, so the prefixed shape names the plugin
explicitly while the bare shape names "every dist-able package sitting at that
version" – a bare tag therefore selects the CLI alone only because the two
numbers are never equal, and there is no way to say "this tag means the CLI
only". `scripts/build-plugin-zip.py --check` refuses a tree where they match.
The CLI's bare shape is not negotiable either: the updater's anchored filter and
every already-installed binary look for `vX.Y.Z`, so a prefixed
`ss-magic-vX.Y.Z` tag would publish a release the whole installed base ignores.

Per-target archives are `ss-magic-<target>.tar.gz` (containing
`ss-magic-<target>/ss-magic`) and `ss-magic-plugin-<target>.tar.gz` (containing
`ss-magic-plugin-<target>/ss-magic-plugin`). Releases are published to GitHub
Releases via cargo-dist (`dist-workspace.toml` for the workspace defaults, plus
per-package `[package.metadata.dist]` tables); only `ss-magic` self-updates from
there. The plugin crate sets `installers = []` – the marketplace is its only
delivery path, so an installer script would be a second, unpinned one – and
declares the plugin zip as its own `[[package.metadata.dist.extra-artifacts]]`
entry with `working-dir = "../.."`, so the zip rides the PLUGIN's tag. Do not
move that entry to workspace level: it would then attach to every release,
the CLI's included. `working-dir` is easy to omit and fails late – a
package-level extra-artifact's build command resolves its working directory
against the CRATE root, so without it cargo-dist looks for the script under
`crates/ss-magic-plugin/scripts/` and fails in `build-global-artifacts`, i.e.
AFTER the tag is pushed and invisibly to `dist plan`. Verify a change there with
`dist build --artifacts=global`, never `dist plan`.

The per-target release archives are attested (cargo-dist `github-attestations` →
`actions/attest` in `build-local-artifacts`, Sigstore/Rekor provenance;
user-facing verification via `gh attestation verify` – see
[README.md](../../README.md), which covers BOTH archive names). The self-update
path trusts TLS + cargo-dist checksums, not attestations. Note the attesting build job
necessarily runs third-party build scripts with `id-token: write` live –
inherent to the feature; the default (build-local) phase is deliberate because
it signs same-job build output before artifacts transit Actions storage, and
changing the phase is a security decision. End-user install instructions (the
installer script and prebuilt-binary download) live in
[README.md](../../README.md); from-source builds and the rest of the contributor
docs (tests, PR expectations, the per-line release procedure) live in
[CONTRIBUTING.md](../../CONTRIBUTING.md).

### The packaged plugin tree and its version surfaces

`plugin/` is the packaged marketplace tree (`.claude-plugin/plugin.json`,
`hooks/hooks.json`, `hooks/bootstrap.sh`, `hooks/run-hook.sh`,
`bin/ss-magic-plugin`, `lib/tmproot.sh`, `lib/execguard.sh`, `skills/` (three
skills: `scratchpad`, `operator-checklist` with its `reference.md`, and
`setup-github-ci`), `ss-magic-plugin.version`); `scripts/build-plugin-zip.py` packs it
byte-reproducibly (sorted entries, fixed 1980-01-01 timestamps, normalized
modes, STORED not deflated, `create_system` forced to unix, `.DS_Store`
excluded, symlinks and non-ASCII names refused loudly), and
`.claude-plugin/marketplace.json` pins the resulting zip by SHA-256.

Version surfaces are GROUPED BY RELEASE LINE, and a surface belongs to exactly
one group:

| Group | Surfaces |
|---|---|
| `ss-magic` | `crates/ss-magic/Cargo.toml`; the `ss-magic` entry in `Cargo.lock`; `README.md`'s pinned installer tag (compared `<=`, not `==`) |
| `ss-magic-plugin` | `crates/ss-magic-plugin/Cargo.toml`; the `ss-magic-plugin` entry in `Cargo.lock`; `plugin/.claude-plugin/plugin.json`; `plugin/ss-magic-plugin.version`; BOTH the tag and the asset name in `marketplace.json`'s release URL; the literal zip filename in the plugin crate's `extra-artifacts` (cargo-dist does not template it) |

The README pin is the one non-equality surface: it names the last PUBLISHED CLI
release, so `--check` requires only that it be a well-formed `v` + triple not
exceeding the crate version. An equality rule would be wrong: the release
procedure is bump → merge → tag, so an equality assertion would make
main's README name an unreleased tag (and 404 the documented install command)
for the whole window between a merged bump and a published release. A lagging
pin names an older release that still works.

Do not work from a remembered count – `--check` enumerates the surfaces and is
the authority. Verify with `python3
scripts/build-plugin-zip.py --check`; after any change under `plugin/`, re-pin
with `--update-manifest` then re-run `--check`. `.gitattributes` marks
`plugin/**` as `-text` so a checkout's line-ending conversion can never move the
digest.
