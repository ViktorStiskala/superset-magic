# Runbook: tag ruleset and release immutability

**Status: NOT APPLIED.** Everything below describes forge settings on
`ViktorStiskala/superset-magic` that a repository administrator must apply by hand. The agent that
wrote this runbook has no authority to change repository settings and deliberately did not try. Tick
the verification section off once a human has applied them.

## Why these two settings exist

The plugin reaches a machine as a **release asset pinned by SHA-256** in
[.claude-plugin/marketplace.json](../../.claude-plugin/marketplace.json). That pin is the only
integrity control on the plugin, and it is only as strong as the immutability of the thing it names.
Two forge-side controls close the gap:

This repository publishes **two release lines** out of one workspace, so both tag shapes matter here:
`v*` for the `ss-magic` sync CLI and `ss-magic-plugin-v*` for the Claude Code plugin. The plugin's zip
and per-target archives ride the second shape; the CLI's installer and archives ride the first.

- **R99 – a tag ruleset.** Without it, a released tag can be force-moved or deleted and recreated, so
  "the plugin at ss-magic-plugin-v1.0.0" stops being a fixed set of bytes.
- **R100 – release immutability.** Without it, a release asset can be **replaced under its existing
  name with the tag untouched** – demonstrated, not theoretical. The marketplace url would then serve
  different bytes, the digest check would fail for every user, and the plugin would simply stop
  installing.

Neither is self-protecting on a personal account: the owner, or any token with classic `repo` scope,
can delete the ruleset or disable immutability. They raise the cost of a mistake and make an
intentional change visible; they are not a boundary against the account owner.

Neither applies retroactively. Releases published before immutability was enabled stay mutable and
are **not** a trust root.

## The release ordering these settings enforce

Two pins move in opposite orders, and the settings below are what make the ordering meaningful.

```mermaid
flowchart TD
  subgraph before["Before the tag is pushed"]
    A["Build plugin/ with scripts/build-plugin-zip.py"] --> B["Commit the digest into marketplace.json"]
    B --> C["Bump every version surface on the line being released (R95, R98)"]
  end
  subgraph tagging["Pushing the tag"]
    C --> D["Push ss-magic-plugin-vX.Y.Z (plugin) or vX.Y.Z (CLI)"]
    D --> E["CI plan phase re-derives the digest and fails on a mismatch"]
    E --> F["cargo-dist publishes that line's assets in one gh release create"]
  end
  subgraph after["After the assets exist"]
    F --> G["Release immutability freezes the assets (R100)"]
    F --> H["Tag ruleset refuses a move or delete of the tag (R99)"]
    F --> I["Only now may plugin/ss-magic-plugin.version advance to X.Y.Z"]
    F --> J["Only now may README's pinned installer tag advance to vX.Y.Z"]
  end
```

The marketplace digest is committed **before** the tag, because the builder can produce it from the
working tree. Between that commit and the release publishing, the entry's `url` names an asset that
does not exist yet; that is expected and self-correcting. The two pins on the right are the opposite:
advancing `plugin/ss-magic-plugin.version` before the named plugin release's assets are published
makes the bootstrap's fetch 404, so nothing installs and every hook fails open with no visible error,
and advancing README's installer tag before the named CLI release exists 404s the documented install
command for everyone who copies it.

The obvious workaround for a mis-cut release – tag, rebuild, commit the new digest, move the tag – is
exactly what the ruleset forbids. GitHub's own documentation is blunt about it: *"Git tags cannot be
moved."* Cut a new patch release instead.

## R99 – the tag ruleset

Apply this exactly. Every field is load-bearing.

| field | value | why |
|---|---|---|
| `target` | `tag` | branches are covered by ordinary branch protection, separately |
| `enforcement` | `active` | `evaluate` reports without blocking, which is not the point |
| `conditions.ref_name.include` | `["~ALL"]` | the two shapes to protect are `refs/tags/v*` and `refs/tags/ss-magic-plugin-v*`, but listing them is **not** enough: the release workflow triggers on `**[0-9]+.[0-9]+.[0-9]+*`, so a `0.9.1` tag with no `v` prefix would still be uncovered. `~ALL` is the superset that covers both shapes and anything version-shaped that slips past them |
| `rules` | `deletion`, `non_fast_forward`, `update` | delete, force-move, and update of an existing tag |
| `creation` | **deliberately absent** | it blocks tag *creation* for the owner too, which breaks releases – a maintainer pushing the tag is what triggers the pipeline |
| `bypass_actors` | `[]` | a bypass actor is a hole in the only control that makes a released tag mean something |

```bash
gh api --method POST repos/ViktorStiskala/superset-magic/rulesets \
  --input - <<'JSON'
{
  "name": "Released tags are immutable",
  "target": "tag",
  "enforcement": "active",
  "bypass_actors": [],
  "conditions": { "ref_name": { "include": ["~ALL"], "exclude": [] } },
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" },
    { "type": "update" }
  ]
}
JSON
```

In the web UI the same thing is Settings → Rules → Rulesets → New ruleset → New tag ruleset, with
the target pattern set to **All tags**, enforcement **Active**, no bypass list, and the three rules
above ticked while **Restrict creations** stays unticked.

### Verifying it (AE83)

```bash
# The ruleset exists, is active, targets every tag, and has no bypass actors.
gh api repos/ViktorStiskala/superset-magic/rulesets \
  --jq '.[] | select(.target=="tag") | {id, name, enforcement}'

gh api repos/ViktorStiskala/superset-magic/rulesets/<id> \
  --jq '{enforcement, bypass: (.bypass_actors|length), include: .conditions.ref_name.include, rules: [.rules[].type]}'
```

Expected: `enforcement: "active"`, `bypass: 0`, `include: ["~ALL"]`, and `rules` containing exactly
`deletion`, `non_fast_forward` and `update`.

Then prove it against a real tag, as the repository owner – the point of the check is that the owner
is not exempt. Run steps 1 and 2 against **both** shapes, since a ruleset scoped to one of them would
pass a check that only ever exercises that one:

```bash
# 1. Deleting a released tag must be refused, on either line.
git push origin :refs/tags/v0.9.0
git push origin :refs/tags/ss-magic-plugin-v1.0.0

# 2. Force-moving a released tag must be refused, on either line.
git tag -f v0.9.0 HEAD && git push --force origin v0.9.0
git tag -f ss-magic-plugin-v1.0.0 HEAD && git push --force origin ss-magic-plugin-v1.0.0

# 3. Creating a NEW tag must still succeed, or the release pipeline is broken.
git tag test-ruleset-creation && git push origin test-ruleset-creation
git push origin :refs/tags/test-ruleset-creation   # this must now be refused too
```

Step 3 leaves a stray tag behind on purpose: with the ruleset active it cannot be deleted, which is
itself the confirmation. Use a name that is obviously disposable and not a version, since the release
workflow only triggers on version-shaped tags.

## R100 – release immutability

```bash
# Enable.
gh api --method PUT repos/ViktorStiskala/superset-magic/immutable-releases

# Confirm.
gh api repos/ViktorStiskala/superset-magic/immutable-releases
```

If that endpoint is not available on the account, the equivalent lives in the web UI under
Settings → General → Releases → **Immutable releases**. Disabling is
`gh api --method DELETE repos/ViktorStiskala/superset-magic/immutable-releases`; record here if it is
ever turned off, and why.

This is compatible with the pipeline as it stands: cargo-dist attaches every asset in the same
`gh release create` call that creates the release, and no later job touches it. The plugin zip rides
that same call as a `[[package.metadata.dist.extra-artifacts]]` entry declared on the **plugin
crate**, so it is frozen with the rest of the plugin release's assets. (Declaring it at workspace
level instead would attach it to every release, the CLI's included – that is why it lives on the
crate.)

One step after the release does not touch assets and is worth knowing about here, because it looks
at first like something immutability would block: after a plugin release, the newest bare `v*`
release has to be re-marked as the repository's latest. That is `gh release edit --latest`, which
changes only the mark and never an asset, so immutability does not conflict with it. The release
workflow does it automatically: cargo-dist's post-announce job `custom-mark-latest` calls
`.github/workflows/mark-latest.yml`, which runs `scripts/mark-latest.sh` and fails loudly if the
mark did not take. The same workflow is `workflow_dispatch`-able as the manual fallback, and
`CONTRIBUTING.md` documents the one-line `gh release edit` alternative.

### Verifying it (AE84)

Substitute a tag that actually exists on each line. At the time of writing the newest published CLI
release is `v0.10.0` and no plugin release has been cut at all, so the second command is the one to
run first; naming an unreleased tag here would fail for a reason that has nothing to do with the
setting under test.

```bash
# Replacing a published asset under its existing name must be refused, on either line.
gh release upload <newest ss-magic-plugin-v* tag> ss-magic-plugin-v<X.Y.Z>.zip --clobber
gh release upload v0.10.0 ss-magic-x86_64-unknown-linux-gnu.tar.gz --clobber
```

Expect a refusal. A release published **before** immutability was enabled will accept this, which is
the non-retroactivity limit stated above rather than a failure of the setting – run the check against
the first release cut after enabling it.

## Restoring these settings

If either is ever removed, this file is the record of what to put back. The ruleset JSON above is
complete and can be re-`POST`ed verbatim; immutability is the single `PUT`. Re-run both verification
sections afterwards, because a ruleset that exists but is set to `evaluate`, or one that acquired a
bypass actor, looks correct in a list and enforces nothing.
