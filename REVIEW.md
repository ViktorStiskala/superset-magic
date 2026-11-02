# Review policy

@CODING_STANDARDS.md
@crates/ss-magic-plugin/CODING_STANDARDS.md
@crates/ss-magic/src/orca/CODING_STANDARDS.md

The coding standards above are what a change is reviewed against; this file adds how findings are ranked and what a finding must carry.

## Severity

- **Blocking** – a defect that cannot be taken back once it ships:
  - secret bytes reaching the shared main checkout without an ignore rule, or entering an archive (an ignore gate verified only in debug builds, an archive that follows a symlink, an exclusion that an ancestor-directory match bypasses);
  - a write that leaves the repository, or that lands through a symlink on a tracked or user-owned file;
  - loss of user bytes or configuration – an overwrite with bytes the review never showed (a source edited during review, a stale status that skips the confirm), an empty archive replacing a good one, a setup command dropped on rewrite;
  - a self-update that silently does nothing;
  - a release published without assets, or anything else that burns a version number (releases are immutable and release tags are protected).
- **Medium** – one file aborting a whole batch or session, a success exit after a file failed, a confirm or summary that hides or misreports what will be or was done (a clipped overwrite list), a backup deleted by the run that wrote it.
- **Low** – wording that misstates nothing material, doc drift, formatting.

Every severity gets fixed: the review history has no declined finding.

## Evidence a finding needs

- A finding names the concrete input or state and the wrong outcome it produces.
