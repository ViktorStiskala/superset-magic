---
title: "feat: GitHub artifact attestations for releases"
date: 2026-07-14
type: feat
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
plan_depth: lightweight
---

# feat: GitHub artifact attestations for releases

## Summary

Enable cargo-dist's native GitHub artifact attestations so every release artifact carries signed, Sigstore-logged build provenance (SLSA v1.0 Build Level 2), verifiable by users with `gh attestation verify`. The change is two lines of TOML plus a regeneration of `release.yml` with the pinned dist v0.32.0, and documentation. Work continues on branch `gh-tests` (open PR #4) — no new branch, per standing instruction. No runtime behavior change → no crate version bump.

**Product Contract preservation:** bootstrapped from the user request; research pre-gathered in `scratchpad/artifact-attestation-findings.md` (session-local) and verified against the pinned dist 0.32.0 binary.

## Requirements

- **R1** — The per-target release archives (`ss-magic-<target>.tar.gz`) are attested: the regenerated release workflow runs an attest step (`actions/attest`) inside `build-local-artifacts` over that job's built archives, producing provenance linked to the workflow, repo, commit SHA, and trigger. (At the default phase the installer script and checksum files are *not* attested — verified against the pinned binary's template; filters only apply at host/announce phases. This is deliberate; see KTD2.)
- **R2** — Regeneration-safe: no hand edits to `release.yml`; `github-attestations = true` in `dist-workspace.toml` + `dist init --yes` (pinned v0.32.0); the `dist plan` drift check passes afterward; hand-authored rationale comments in `dist-workspace.toml` are restored post-regeneration.
- **R3** — Permission hygiene: the regeneration adds a per-job permissions block (`attestations: write`, `contents: read`, `id-token: write`) on `build-local-artifacts` and leaves the workflow-level block untouched (verified against the pinned binary's template). Acceptance covers both shapes dist could produce: per-job grants (expected), or workflow-level grants — the latter acceptable only because `ci.yml`'s own workflow-level `permissions: contents: read` caps the reusable-workflow token via GitHub's downgrade-only intersection; that ci.yml block, not caller-side scoping, is the load-bearing guard for `custom-ci` and must remain unchanged.
- **R4** — Users can verify: README gains a "Verify a release" subsection (`gh attestation verify <artifact> -R ViktorStiskala/superset-magic`, public repo → Sigstore Public Good + Rekor transparency log, provenance ≠ safety caveat); CLAUDE.md's build/release notes mention attestations alongside the checksum-based self-update integrity story; `.cursor/BUGBOT.md` stays synchronized per its self-containment convention.

## Key Technical Decisions

- **KTD1 — Config-driven, never hand-edited.** Identical procedure to the `plan-jobs` change already on this branch: config key → `dist init --yes` → inspect diff → restore comments. The pinned binary verifiably supports `github-attestations`, `github-attestations-filters`, and `github-attestations-phase` and references `actions/attest`.
- **KTD2 — Keep the default phase (`build-local-artifacts`) as a deliberate security choice; accept archive-only coverage.** The default phase attests only the per-target `.tar.gz` archives, in the same job that built them — the attest step's `subject-path` references `target/distrib/*` (same-job build output), so the signed bytes never transit Actions artifact storage before signing. Moving to `github-attestations-phase = "host"` would widen coverage to `artifacts/*` (installer + checksums) but signs a `download-artifact` merge directory that any job in the run can inject into via the runtime token — converting a detectable injection (unattested file) into a cryptographically endorsed one. Phase changes are therefore a security decision requiring explicit review, never a diff-reconciliation judgment call. No `github-attestations-filters` (inert at the default phase). Trade-off accepted: the installer script and checksums stay unattested; the installer's integrity path remains TLS + the checksummed archives it downloads.
- **KTD3 — No version bump.** The binary's behavior is unchanged; attestations are a release-pipeline/user-verification feature. A release must be cut later for attestations to exist on real artifacts — out of scope here.
- **KTD4 — Self-updater unchanged.** `self_update` integrity remains TLS + cargo-dist checksums; attestation verification inside the updater is a noted future consideration, not in scope.

## Implementation Units

### U1. Enable attestations and regenerate release.yml

**Goal:** `dist-workspace.toml` gains `github-attestations = true`; `release.yml` is regenerated with the attest wiring.

**Requirements:** R1, R2, R3.

**Files:** `dist-workspace.toml`, `.github/workflows/release.yml` (regenerated).

**Approach:** Add the key with a short rationale comment under `[dist]`. Run `~/.cargo/bin/dist init --yes` (v0.32.0 already installed this session), then: (a) restore any stripped rationale comments in `dist-workspace.toml` (unix-archive, install-updater, plan-jobs notes); (b) inspect the `release.yml` diff — expect an `actions/attest` step with `subject-path` over the artifacts and per-job `id-token: write` + `attestations: write`; confirm the `custom-ci` job's call is unchanged (its own `permissions: contents: read` in ci.yml caps the token regardless — the workflow-level permission intersection is the guard); (c) if the diff shows unrelated drift, stop and reconcile. Run `dist plan` as the drift check.

**Test scenarios:** Test expectation: none — release config; verified structurally by diff inspection + drift check, and live on the next tag.

**Verification:** Regenerated diff contains the attest step and correctly scoped permissions; `dist plan` exits 0; `custom-ci` job wiring untouched.

### U2. Document verification

**Goal:** Users and maintainers know attestations exist and how to verify them.

**Requirements:** R4.

**Dependencies:** U1.

**Files:** `README.md`, `CLAUDE.md`, `.cursor/BUGBOT.md`.

**Approach:** README: "Verify a release" subsection near the install instructions, scoped to the `.tar.gz` archives explicitly (the verify example targets a downloaded archive; the installer script and checksum files are not attested — say so), version-scoped ("releases from v0.2.0 onward"; latest published tag is v0.1.1, so the command has no target until the next release), one line on Sigstore/Rekor (public repo), and the caveat that attestations prove provenance, not safety. Keep wording compatible with the "Verifying GitHub Artifact Attestations" section dist auto-adds to generated release notes (`--bundle` variant) — two valid commands, don't contradict. CLAUDE.md: extend the Build section note — release archives are attested via cargo-dist's `github-attestations`; self-update integrity remains checksums (attestations are user-facing); mirror the token-hygiene note so the ci.yml rationale and the attesting-build-job reality don't read as contradictory. BUGBOT.md: one rule — flag removal of `github-attestations`, a phase change away from `build-local-artifacts` without security review, or attestation steps stripped from the release workflow.

**Test scenarios:** Test expectation: none — docs.

**Verification:** Docs mention the exact verify command and repo slug; BUGBOT.md remains self-contained.

## Scope Boundaries

**Out of scope:** cutting the release that produces the first attested artifacts; attestation verification inside the self-updater (future consideration); `github-attestations-filters` tuning; branch protection.

## Assumptions

*(Headless run.)* **A1 — resolved during doc review:** the pinned binary's template verifiably adds a per-job permissions block (`attestations: write`, `contents: read`, `id-token: write`) on `build-local-artifacts` and leaves the workflow-level block untouched; diff inspection confirms, stop-and-reconcile remains the guard. **A2** — Default attestation phase is kept as a security decision (KTD2), with archive-only coverage accepted. **A3** — Staying on `gh-tests`/PR #4 is explicitly requested; attestation ships in the same PR as the CI/gating work.

## Risks

- **Regeneration drift (low):** same pinned-version procedure already exercised twice on this branch.
- **First live attestation happens on the next tag (accepted):** the PR can only validate structure (drift check + PR-triggered plan job); the attest step itself runs only in a real release. A Sigstore/`actions-attest` outage would fail a release at tag time with zero pre-merge signal; rollback is cheap (`github-attestations = false` + regenerate). Mitigation: KTD2 defaults + cargo-dist's own tested template.
- **OIDC minting lives in a job that executes third-party build scripts (accepted, inherent):** the attesting build job compiles all dependencies (build.rs), so a compromised dependency could request the runner's OIDC token and mint Rekor-permanent attestations under this repo's identity. This is inherent to the feature at every phase; incremental harm is bounded (a compromised build script already controls the released bytes, and an honestly-attested trojan passes verification anyway — the provenance ≠ safety caveat). Build-local remains the least-bad phase (signs before artifact-storage transit).
- **Experimental flag (informational):** cargo-dist 0.32 marks `github-attestations` experimental; behavior is pin-verified against the installed binary.

## Verification Contract

1. `dist plan` passes post-regeneration; `custom-ci` wiring unchanged; **ci.yml's workflow-level `permissions: contents: read` block unchanged** (it, not release.yml, is what protects the test job's token).
2. Regenerated `release.yml` contains an `actions/attest` step in `build-local-artifacts` whose `subject-path` references same-job build output (`target/distrib/*`), **never** a `download-artifact` destination; the permissions grant matches R3's accepted shapes.
3. The attest step is unreachable on `pull_request`-triggered runs (build jobs stay gated on `publishing == 'true'` with no `pr_run_mode` configured).
4. Global artifacts (installer script, checksums) confirmed excluded from attestation at this phase — and the README says so (no "all release artifacts" claim anywhere).
5. README/CLAUDE.md/BUGBOT.md updated; no crate version change; `cargo test --locked` still green (223 tests — untouched by this change).

## Definition of Done

Both units landed on `gh-tests`; Verification Contract satisfied; PR #4 CI green on the new head.

## Sources & Research

- `scratchpad/artifact-attestation-findings.md` (research fork, this session): GitHub artifact-attestations docs, cargo-dist attestations book page, pinned-binary capability verification (`github-attestations*` keys, `actions/attest` reference), repo eligibility (public → Sigstore Public Good).
- This branch's prior `plan-jobs` regeneration procedure (PR #4) — same drift-check constraints.
