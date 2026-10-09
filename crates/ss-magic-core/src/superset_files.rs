//! Read and write the `.superset/` workspace contract:
//!
//!   .superset/config.json         { setup, teardown, run }  (Superset-owned)
//!   .superset/magic.sh            executable wrapper (embedded asset)
//!   .superset/setup_config.json   { files: [pattern, ...] }  (legacy; read-only)
//!   .superset/magic.json          { files: [pattern, ...] }  (committed)
//!   .superset/magic.local.json    { files: [pattern, ...] }  (gitignored overlay)
//!   .superset/config.local.json   Superset's per-machine override of config.json
//!                                 (gitignored; a local install registers
//!                                 `ss-magic sync` in its `setup` key)
//!
//! The embedded `magic.sh` (under `assets/`) is the single source of truth
//! for the wrapper body; migration and init write the on-disk copy. The
//! legacy `setup_config.json` is still read by migration (to carry its
//! `files` into `magic.json`) but never written.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Embedded canonical body of `magic.sh`. Written to `.superset/magic.sh`
/// during migration (U9) and bootstrap. Delegates to `ss-magic` via `exec`
/// when the binary is available; otherwise prints an install hint and exits 0
/// so Superset's setup pipeline continues uninterrupted.
pub const MAGIC_SH: &str = include_str!("../../../assets/magic.sh");

const SUPERSET_DIR: &str = ".superset";
const CONFIG_JSON: &str = "config.json";
const MAGIC_SH_NAME: &str = "magic.sh";
const SETUP_CONFIG_JSON: &str = "setup_config.json";
const MAGIC_JSON: &str = "magic.json";
const MAGIC_LOCAL_JSON: &str = "magic.local.json";
const CONFIG_LOCAL_JSON: &str = "config.local.json";

/// Relative path of `magic.local.json` as it appears inside the repo – the ONE
/// spelling of it. Seeded as a sync pattern by [`default_magic_files`] and
/// [`local_install_default_files`], and imported by the CLI wherever it names
/// the file: the init/migrate bootstrap gitignore rule and the local install's
/// tracked-file refusal, ignore rule and progress lines.
pub const MAGIC_LOCAL_PATTERN: &str = ".superset/magic.local.json";

/// Relative path of `config.local.json` as it appears inside the repo – the ONE
/// spelling of it. A local install lists it as a sync pattern so forward sync
/// carries Superset's per-machine setup override into every worktree, and the
/// CLI's reverse-sync forward-only guard (`is_forward_only_rel`) compares
/// against this same constant, so the pattern that is seeded and the path that
/// is never pushed back into main cannot drift apart.
pub const CONFIG_LOCAL_PATTERN: &str = ".superset/config.local.json";

/// The literal setup command a local install registers in `config.local.json`.
/// Superset runs it directly (a local install writes no `magic.sh` wrapper, as
/// the wrapper would be a committed file). The CLI's committed-install detector
/// (`entry_is_magic_marker`) and [`setup_has_sync_marker`] both match on this
/// constant, so the entry written and the entry recognized cannot drift apart.
pub const LOCAL_SYNC_ENTRY: &str = "ss-magic sync";

/// Shape of `.superset/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub teardown: Vec<String>,
    #[serde(default)]
    pub run: Vec<String>,
}


/// Shape of the legacy `.superset/setup_config.json`. Read-only: migration
/// reads its `files` to carry them into `magic.json`. Never written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetupConfig {
    #[serde(default)]
    pub files: Vec<String>,
}

/// Shape of `.superset/magic.json` (committed) and `.superset/magic.local.json`
/// (gitignored local overlay).
///
/// Named fields hold the keys this version of ss-magic actually understands
/// (currently just `files`); everything else lands in `extras` (KTD8) via
/// `#[serde(flatten)]` instead of being dropped. That matters because a
/// `magic.json` written by a NEWER ss-magic can carry keys this build has
/// never heard of (e.g. a future `plugin` block) — every writer must
/// round-trip those keys unchanged rather than silently deleting
/// configuration it doesn't recognize. Add a new known key as its own named
/// field rather than reaching into `extras` for it, so future units (typing
/// a `plugin` field, `config set`/`enable`/`disable`) build on this
/// unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MagicConfig {
    /// Glob patterns for files to sync from main into worktrees.
    #[serde(default)]
    pub files: Vec<String>,
    /// Every top-level key this build of ss-magic has no named field for.
    /// Serialized back out verbatim alongside `files` on every write.
    #[serde(flatten)]
    pub extras: serde_json::Map<String, serde_json::Value>,
}

/// Read and overlay `.superset/magic.json` with `.superset/magic.local.json`.
///
/// Overlay rules:
/// - `files`: UNION + DEDUPE — magic.json order is preserved first; local
///   entries not already present are appended in local order. A local entry
///   that duplicates a base entry is silently dropped (base position kept).
/// - Scalar / object keys (future): local value wins.
/// - Missing base `magic.json` → `Ok(None)`.
/// - Malformed `magic.json` OR malformed `magic.local.json` → hard error
///   naming the offending path; no silent fallback.
/// - Missing `magic.local.json` → base only.
pub fn load_overlaid(root: &Path) -> Result<Option<MagicConfig>> {
    let base_path = superset_dir(root).join(MAGIC_JSON);
    let local_path = superset_dir(root).join(MAGIC_LOCAL_JSON);

    // Missing base → None (not an error).
    let base: MagicConfig = match read_json::<MagicConfig>(&base_path)
        .with_context(|| format!("reading {}", base_path.display()))?
    {
        None => return Ok(None),
        Some(cfg) => cfg,
    };

    // Missing local → use base as-is.
    let local: Option<MagicConfig> = read_json::<MagicConfig>(&local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;

    let Some(local) = local else {
        return Ok(Some(base));
    };

    // Merge: union + dedupe files (base order first, then new local entries).
    let mut merged_files = base.files.clone();
    for entry in &local.files {
        if !merged_files.iter().any(|e| e == entry) {
            merged_files.push(entry.clone());
        }
    }

    // Merge extras (KTD8) per the rule documented above: local's value wins
    // per key; a key present only in base is kept as-is.
    let mut merged_extras = base.extras.clone();
    for (key, value) in &local.extras {
        merged_extras.insert(key.clone(), value.clone());
    }

    Ok(Some(MagicConfig {
        files: merged_files,
        extras: merged_extras,
    }))
}

/// Rewrite `.superset/magic.json` from `cfg` (including whatever it carries
/// in `extras`), pretty-printed with a trailing newline.
///
/// This is the plain writer — it serializes exactly what `cfg` holds. It does
/// NOT itself preserve anything: a caller updating `files` on top of an
/// existing on-disk file must first load that file and carry its `extras`
/// forward (see [`merge_files_into_magic_config`]), the same
/// load-modify-write discipline `merge_setup_into_config` already uses for
/// `config.json`.
pub fn write_magic_json(root: &Path, cfg: &MagicConfig) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(MAGIC_JSON);
    let body = format!("{}\n", serde_json::to_string_pretty(cfg)?);
    write_atomically(&path, &body)
}

/// Rewrite `.superset/magic.local.json` from `cfg` (including whatever it
/// carries in `extras`), pretty-printed with a trailing newline.
///
/// The gitignored counterpart to [`write_magic_json`], and equally a plain
/// writer: it serializes exactly what `cfg` holds and preserves nothing on
/// its own. A caller changing one key on top of an existing local file (the
/// `--local` half of `enable`/`disable`/`config set`, R7) must first load
/// that file with [`load_magic_local_json`] and carry its `extras` forward,
/// same as every other writer in this module.
pub fn write_magic_local_json(root: &Path, cfg: &MagicConfig) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(MAGIC_LOCAL_JSON);
    let body = format!("{}\n", serde_json::to_string_pretty(cfg)?);
    write_atomically(&path, &body)
}

/// Replace `path` with `body` in one step: write a private sibling file, then
/// rename it over the target.
///
/// Both `magic.json` writers used to be a plain `fs::write`, which truncates
/// the file first and fills it afterwards. That was fine while one person
/// ran one command at a time, but `magic.json` now has an unattended writer
/// too: the plugin's `seed-config` runs from every session start, so a
/// session that dies mid-write (a hook timeout, a closed terminal) could
/// leave the TRACKED file that also holds the sync patterns truncated, and
/// two writers could interleave their bytes. A rename is atomic on the
/// filesystems git itself relies on, so a reader sees either the previous
/// file or the whole new one, never a prefix. The sibling carries the
/// process id and a per-process counter so concurrent writers stage
/// different files; whichever renames last wins whole, which is the same
/// outcome a sequential pair of writes has.
fn write_atomically(path: &Path, body: &str) -> Result<()> {
    // Write THROUGH a symlink, as `fs::write` did: rename onto the link's
    // resolved target, not onto the link itself, which would silently turn
    // the link into a plain file and leave its target stale. The plugin's
    // seed has already decided whether that target is acceptable (it refuses
    // one that leaves the repository); a target that does not exist yet
    // resolves to the path as given.
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = target.as_path();
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("{} has no file name", path.display()))?;
    // Unique per writer, not merely per process: two threads of one process
    // (the concurrent-write test is exactly that) must stage different files.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staged = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    let write = fs::write(&staged, body)
        .with_context(|| format!("writing {}", staged.display()))
        .and_then(|()| {
            fs::rename(&staged, path)
                .with_context(|| format!("moving {} over {}", staged.display(), path.display()))
        });
    if write.is_err() {
        // Best-effort: a failed write must not leave its staging file behind
        // as an untracked stranger in the user's `.superset/`.
        let _ = fs::remove_file(&staged);
    }
    write
}

/// Build a fresh `MagicConfig` with `new_files`, carrying forward the
/// unknown top-level keys (KTD8) from `existing`, if any. Mirrors
/// `merge_setup_into_config`'s preservation discipline for `config.json`:
/// every write path (init, migrate, edit-config) loads the current on-disk
/// `magic.json`, calls this to change just `files`, then writes the result —
/// never rebuilding a `MagicConfig` from parts alone, which would silently
/// drop any key this build doesn't have a named field for.
pub fn merge_files_into_magic_config(
    existing: Option<&MagicConfig>,
    new_files: Vec<String>,
) -> MagicConfig {
    let extras = match existing {
        Some(cfg) => cfg.extras.clone(),
        None => serde_json::Map::new(),
    };
    MagicConfig {
        files: new_files,
        extras,
    }
}

/// Default patterns included in every freshly-written `magic.json`.
///
/// Contains `.superset/magic.local.json` so forward sync copies the local
/// overlay into each worktree.  Consumed by the init/migration unit (U9)
/// when it writes `magic.json` for the first time.
// consumed by U9
#[allow(dead_code)]
pub fn default_magic_files() -> Vec<String> {
    vec![MAGIC_LOCAL_PATTERN.to_string()]
}

/// Bootstrap `.superset/magic.local.json` if it does not already exist.
///
/// Writes a strict JSON object with a `_comment` string key (serde
/// round-trips it) and an empty `files` array.  The comment explains
/// that the file is gitignored and acts as the local overlay.
///
/// Idempotent: does nothing when the file already exists.
// consumed by U9
#[allow(dead_code)]
pub fn bootstrap_magic_local_json(root: &Path) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(MAGIC_LOCAL_JSON);
    if path.exists() {
        return Ok(());
    }
    // Write raw JSON so the _comment key is included without requiring a
    // corresponding struct field on MagicConfig.  serde ignores unknown
    // keys on deserialisation, so load_overlaid round-trips this as empty.
    let body = "{\n  \"_comment\": \"Local overlay for magic.json — gitignored, never committed. Add patterns here that are specific to this machine or checkout.\",\n  \"files\": []\n}\n";
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Which kind of ss-magic install a checkout carries. Derived from the files
/// on disk by [`install_mode`], never stored: a stored flag could disagree with
/// the files, and the files are what sync actually reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    /// `.superset/magic.json` exists: the team-visible (committed) install.
    /// Its pattern list is the overlay of `magic.json` and `magic.local.json`.
    Committed,
    /// No `magic.json`, but `.superset/magic.local.json` exists: a local
    /// (uncommitted) install whose only pattern list is the local file.
    Local,
    /// Neither file exists: no ss-magic install in this checkout.
    None,
}

/// Classify the install in `root` from its `.superset/` files alone.
///
/// - `magic.json` exists: [`InstallMode::Committed`], even when
///   `magic.local.json` also exists (a local overlay on a committed install is
///   the ordinary committed shape, and committed must win so the overlay keeps
///   its union semantics).
/// - `magic.json` absent, `magic.local.json` present: [`InstallMode::Local`].
/// - Otherwise [`InstallMode::None`].
///
/// The `ss-magic sync` marker in `config.local.json` deliberately does NOT
/// decide the mode: it only makes the setup merge idempotent and drives the
/// duplicate-entry warning. Existence is tested with `exists()`, not
/// `is_file()`, so a `magic.json` that is a directory still selects
/// `Committed` and the loader then fails loudly instead of the checkout being
/// silently reclassified as a local install.
pub fn install_mode(root: &Path) -> InstallMode {
    if superset_dir(root).join(MAGIC_JSON).exists() {
        InstallMode::Committed
    } else if superset_dir(root).join(MAGIC_LOCAL_JSON).exists() {
        InstallMode::Local
    } else {
        InstallMode::None
    }
}

/// Load the pattern list sync, reverse sync and pack should use for `root`.
///
/// Follows [`install_mode`]: the overlay of `magic.json` and `magic.local.json`
/// for a committed install ([`load_overlaid`], unchanged), `magic.local.json`
/// alone for a local install, and `Ok(None)` when there is no install. A
/// malformed file is a hard error naming the path, as in [`load_overlaid`].
///
/// [`load_overlaid`] keeps its exact behavior (it answers `None` without a
/// `magic.json`) because the plugin crate calls it; the CLI uses this loader
/// instead wherever a local install must be accepted.
pub fn load_sync_config(root: &Path) -> Result<Option<MagicConfig>> {
    match install_mode(root) {
        InstallMode::Committed => load_overlaid(root),
        InstallMode::Local => load_magic_local_json(root),
        InstallMode::None => Ok(None),
    }
}

/// The patterns a fresh local install seeds into `magic.local.json`: the local
/// pattern file itself and Superset's `config.local.json`, so forward sync
/// copies both into every worktree. The user's chosen patterns follow them.
pub fn local_install_default_files() -> Vec<String> {
    vec![MAGIC_LOCAL_PATTERN.to_string(), CONFIG_LOCAL_PATTERN.to_string()]
}

/// Whether a `setup` value (any JSON shape) already registers `ss-magic sync`.
///
/// Walks strings, arrays and objects (so both Superset forms are covered: the
/// plain array that replaces the committed key and the `{before, after}`
/// object that wraps it) and reports true when any string contains
/// [`LOCAL_SYNC_ENTRY`]. Numbers, booleans and null never match.
pub fn setup_has_sync_marker(setup: &serde_json::Value) -> bool {
    match setup {
        serde_json::Value::String(s) => s.contains(LOCAL_SYNC_ENTRY),
        serde_json::Value::Array(items) => items.iter().any(setup_has_sync_marker),
        serde_json::Value::Object(map) => map.values().any(setup_has_sync_marker),
        _ => false,
    }
}

/// Result of [`merge_sync_entry_into_local_config`]: the document to write and
/// whether it differs from the input. A caller skips the write when `changed`
/// is false, so a re-run never rewrites the file.
#[derive(Debug, Clone)]
pub struct LocalConfigMerge {
    pub config: serde_json::Map<String, serde_json::Value>,
    pub changed: bool,
}

/// Load `.superset/config.local.json` as a raw JSON object. `Ok(None)` when
/// absent; an error naming the path when it is malformed or is not a JSON
/// object. It is Superset's file, not ours, so it is kept as an untyped map:
/// every key (including ones Superset adds later) survives a rewrite.
pub fn load_config_local_json(
    root: &Path,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
    let path = superset_dir(root).join(CONFIG_LOCAL_JSON);
    read_json::<serde_json::Map<String, serde_json::Value>>(&path)
        .with_context(|| format!("reading {}", path.display()))
}

/// Add the `ss-magic sync` setup entry to a `config.local.json` document,
/// without touching anything else in it.
///
/// Superset merges this file over the committed `config.json`: a key that is a
/// plain array REPLACES the committed one, and a key that is a `{before,
/// after}` object WRAPS it. The entry goes where it runs first without
/// dropping the team's steps (so files such as `.env` exist before
/// `bun install` or a migration runs):
///
/// - `setup` absent: `{"before": ["ss-magic sync"]}`.
/// - `setup` an object: the entry is prepended to its `before` array, which is
///   created when missing; `after` and any other key are untouched.
/// - `setup` a plain array (the replace form): the entry is prepended to that
///   array, keeping the form the user chose.
/// - the marker already appears anywhere in `setup` ([`setup_has_sync_marker`]):
///   nothing changes and `changed` is false.
/// - `setup` of any other JSON type (string, number, bool, null), or an object
///   whose `before` is not an array: refused with an error rather than
///   overwritten, because guessing at a shape Superset may interpret
///   differently could discard the user's own setup.
///
/// Every other top-level key (`teardown`, `run`, unknown ones) keeps its value.
/// The map is serde_json's default sorted map, so a rewritten file orders its
/// keys alphabetically; values survive, and an unchanged document is never
/// rewritten.
pub fn merge_sync_entry_into_local_config(
    existing: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<LocalConfigMerge> {
    use serde_json::Value;

    let mut config = existing.cloned().unwrap_or_default();
    let entry = || Value::String(LOCAL_SYNC_ENTRY.to_string());

    let Some(setup) = config.get_mut("setup") else {
        config.insert(
            "setup".to_string(),
            serde_json::json!({ "before": [LOCAL_SYNC_ENTRY] }),
        );
        return Ok(LocalConfigMerge { config, changed: true });
    };

    if setup_has_sync_marker(setup) {
        return Ok(LocalConfigMerge { config, changed: false });
    }

    match setup {
        Value::Array(items) => items.insert(0, entry()),
        Value::Object(map) => match map.get_mut("before") {
            None => {
                map.insert("before".to_string(), Value::Array(vec![entry()]));
            }
            Some(Value::Array(items)) => items.insert(0, entry()),
            Some(other) => bail!(
                "`setup.before` in {CONFIG_LOCAL_JSON} is {}, not an array; fix it by hand, then re-run",
                json_type_name(other)
            ),
        },
        other => bail!(
            "`setup` in {CONFIG_LOCAL_JSON} is {}, not an array or a {{before, after}} object; \
             fix it by hand, then re-run",
            json_type_name(other)
        ),
    }
    Ok(LocalConfigMerge { config, changed: true })
}

/// A short article-plus-noun name for a JSON value's type, for error messages.
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Rewrite `.superset/config.local.json` from `cfg`, pretty-printed with a
/// trailing newline, through the same staged-sibling-plus-rename commit as the
/// `magic*.json` writers (a write that dies half-way leaves the previous file).
///
/// A plain writer: it serializes exactly what `cfg` holds. A caller changing
/// one key must load the file with [`load_config_local_json`] first and carry
/// the rest forward, as [`merge_sync_entry_into_local_config`] does.
pub fn write_config_local_json(
    root: &Path,
    cfg: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(CONFIG_LOCAL_JSON);
    let body = format!("{}\n", serde_json::to_string_pretty(cfg)?);
    write_atomically(&path, &body)
}

fn superset_dir(root: &Path) -> PathBuf {
    root.join(SUPERSET_DIR)
}

/// Load just `config.json` from `root/.superset/`. `Ok(None)` when the
/// file is absent; error when it exists but cannot be parsed.
pub fn load_config(root: &Path) -> Result<Option<Config>> {
    read_json::<Config>(&superset_dir(root).join(CONFIG_JSON)).with_context(|| {
        format!("reading {}", superset_dir(root).join(CONFIG_JSON).display())
    })
}

/// Load just `magic.json` (the committed base) from `root/.superset/`.
/// `Ok(None)` when the file is absent; error when it exists but cannot be
/// parsed. Does NOT read or merge `magic.local.json` — use [`load_overlaid`]
/// when you want the full union.
pub fn load_magic_json(root: &Path) -> Result<Option<MagicConfig>> {
    read_json::<MagicConfig>(&superset_dir(root).join(MAGIC_JSON)).with_context(|| {
        format!(
            "reading {}",
            superset_dir(root).join(MAGIC_JSON).display()
        )
    })
}

/// Load just `magic.local.json` (the gitignored per-machine overlay) from
/// `root/.superset/`. `Ok(None)` when the file is absent; error when it
/// exists but cannot be parsed. Does NOT read or merge `magic.json` — use
/// [`load_overlaid`] when you want the full union.
pub fn load_magic_local_json(root: &Path) -> Result<Option<MagicConfig>> {
    read_json::<MagicConfig>(&superset_dir(root).join(MAGIC_LOCAL_JSON)).with_context(|| {
        format!(
            "reading {}",
            superset_dir(root).join(MAGIC_LOCAL_JSON).display()
        )
    })
}

/// Load just `setup_config.json` from `root/.superset/`. `Ok(None)` when
/// the file is absent; error when it exists but cannot be parsed.
pub fn load_setup_config(root: &Path) -> Result<Option<SetupConfig>> {
    read_json::<SetupConfig>(&superset_dir(root).join(SETUP_CONFIG_JSON)).with_context(|| {
        format!(
            "reading {}",
            superset_dir(root).join(SETUP_CONFIG_JSON).display()
        )
    })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        bail!("`{}` exists but is not a regular file", path.display());
    }
    let raw = fs::read_to_string(path)?;
    let parsed = serde_json::from_str::<T>(&raw)
        .with_context(|| format!("malformed JSON in {}", path.display()))?;
    Ok(Some(parsed))
}

/// Create `.superset/` if missing; error if it exists as a non-directory.
pub fn ensure_superset_dir(root: &Path) -> Result<()> {
    let dir = superset_dir(root);
    if dir.exists() {
        if !dir.is_dir() {
            bail!(
                "`{}` exists but is not a directory; remove or rename it before running bootstrap",
                dir.display()
            );
        }
        return Ok(());
    }
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(())
}

/// Always overwrite `.superset/magic.sh` with the embedded canonical body
/// and mark it executable (mode 0755).
///
/// The wrapper delegates to `ss-magic` via `exec` when the binary is on
/// `PATH`, and exits 0 with an install hint when it is absent — so
/// Superset's setup pipeline always continues. Called by the migration and
/// init flows (U9); wired there rather than here.
// consumed by U9
#[allow(dead_code)]
pub fn write_magic_sh(root: &Path) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(MAGIC_SH_NAME);
    fs::write(&path, MAGIC_SH).with_context(|| format!("writing {}", path.display()))?;
    chmod_executable(&path)?;
    Ok(())
}

#[cfg(unix)]
fn chmod_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).with_context(|| format!("chmod 0755 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn chmod_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Always rewrite `.superset/config.json` from `cfg`, pretty-printed with
/// a trailing newline. Preservation of pre-existing `teardown` / `run`
/// arrays happens upstream of this call by way of [`merge_setup_into_config`].
pub fn write_config_json(root: &Path, cfg: &Config) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(CONFIG_JSON);
    let body = format!("{}\n", serde_json::to_string_pretty(cfg)?);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Build a fresh `Config` from `new_setup` while preserving the existing
/// on-disk `teardown` and `run` arrays. When `existing` is `None`, both
/// fall back to empty vectors.
pub fn merge_setup_into_config(existing: Option<&Config>, new_setup: Vec<String>) -> Config {
    let (teardown, run) = match existing {
        Some(cfg) => (cfg.teardown.clone(), cfg.run.clone()),
        None => (Vec::new(), Vec::new()),
    };
    Config {
        setup: new_setup,
        teardown,
        run,
    }
}

/// Copy the staged `.superset` tree from `stage_root` into `repo_root` and
/// delete any repo-relative paths named in `delete`, applying the
/// preservation rules:
///
/// - Every regular file present in the staged `.superset/` directory is
///   copied over the matching path under `repo_root/.superset/`. Files are
///   always overwritten — preservation (e.g. of `config.json`'s existing
///   `teardown` / `run` arrays) must happen upstream by merging into the
///   staged tree before this call. Any staged `*.sh` is chmod 0755'd after
///   the copy so the `magic.sh` wrapper stays executable.
/// - Each repo-relative path in `delete` (e.g. `.superset/setup.sh`) is
///   removed from `repo_root` if it exists. Used by migration to strip the
///   retired `setup.sh`. A missing target is not an error.
///
/// The staged tree is the source of truth: migration stages `magic.sh` +
/// `magic.json` + `config.json` (+ `magic.local.json`) and asks for
/// `.superset/setup.sh` to be deleted.
///
/// ## Invariant (KTD2): never prunes an unnamed destination entry
///
/// This function never removes anything under `repo_root/.superset/` that
/// isn't named in `delete`. It only ever touches two things: the staged
/// files it copies IN, and the explicit `delete` list. It never enumerates
/// `repo_root/.superset/` itself and removes what it finds missing from the
/// stage — so anything already living there that the stage and `delete`
/// don't mention is left completely alone. Today that holds by construction
/// (the loop below reads only the STAGE directory's flat file list, never
/// the destination's), but it matters because `.superset/.magic/` — the
/// Claude plugin's session state, conclusion cache, and one-shot claims —
/// lives *inside* the very destination root this function owns, and is never
/// staged or named in `delete` by any caller. A future change that walked
/// `repo_root/.superset/` and pruned whatever it didn't recognize would
/// silently delete that live state on the next `init` or `migrate`, with no
/// error and no warning. State this explicitly so a later edit has to break
/// it on purpose, not by accident.
pub fn copy_into_repo(stage_root: &Path, repo_root: &Path, delete: &[&str]) -> Result<()> {
    ensure_superset_dir(repo_root)?;
    let stage_dir = stage_root.join(SUPERSET_DIR);
    let real_dir = repo_root.join(SUPERSET_DIR);

    // Collect the staged files (the tree is flat — no subdirectories under
    // `.superset/`), then copy them with `config.json` LAST. config.json is
    // the file Superset reads to locate the wrapper, so writing it last means
    // a mid-copy failure can never leave config.json pointing at a `magic.sh`
    // that hasn't been written yet — no half-migrated tree with a live pointer
    // to a missing target.
    let mut staged: Vec<(std::ffi::OsString, std::path::PathBuf)> = Vec::new();
    let entries = fs::read_dir(&stage_dir)
        .with_context(|| format!("reading staged dir {}", stage_dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry in {}", stage_dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if file_type.is_file() {
            staged.push((entry.file_name(), entry.path()));
        }
    }
    // false (every other file) sorts before true (config.json) → config last.
    staged.sort_by_key(|(name, _)| name.as_os_str() == std::ffi::OsStr::new(CONFIG_JSON));
    for (name, src) in &staged {
        let dst = real_dir.join(name);
        fs::copy(src, &dst)
            .with_context(|| format!("copy {} → {}", src.display(), dst.display()))?;
        // Keep shell wrappers executable.
        if Path::new(name).extension().is_some_and(|ext| ext == "sh") {
            chmod_executable(&dst)?;
        }
    }

    // Delete retired repo-relative paths (e.g. `.superset/setup.sh`).
    for rel in delete {
        let target = repo_root.join(rel);
        if target.exists() {
            fs::remove_file(&target)
                .with_context(|| format!("deleting {}", target.display()))?;
        }
    }

    Ok(())
}

/// Entries from `existing` that are NOT in `options`, in their original
/// order. Used to preserve user-typed entries (patterns or commands)
/// across edit-mode re-runs.
pub fn existing_unknown_entries(existing: &[String], options: &[&str]) -> Vec<String> {
    existing
        .iter()
        .filter(|p| !options.iter().any(|o| o == &p.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
