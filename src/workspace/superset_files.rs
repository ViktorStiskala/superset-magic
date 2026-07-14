//! Read and write the `.superset/` workspace contract:
//!
//!   .superset/config.json         { setup, teardown, run }  (Superset-owned)
//!   .superset/magic.sh            executable wrapper (embedded asset)
//!   .superset/setup_config.json   { files: [pattern, ...] }  (legacy; read-only)
//!   .superset/magic.json          { files: [pattern, ...] }  (committed)
//!   .superset/magic.local.json    { files: [pattern, ...] }  (gitignored overlay)
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
pub const MAGIC_SH: &str = include_str!("../../assets/magic.sh");

const SUPERSET_DIR: &str = ".superset";
const CONFIG_JSON: &str = "config.json";
const MAGIC_SH_NAME: &str = "magic.sh";
const SETUP_CONFIG_JSON: &str = "setup_config.json";
const MAGIC_JSON: &str = "magic.json";
const MAGIC_LOCAL_JSON: &str = "magic.local.json";

/// Relative path of `magic.local.json` as it appears inside the repo.
/// Referenced by [`default_magic_files`] and the bootstrap helper.
const MAGIC_LOCAL_PATTERN: &str = ".superset/magic.local.json";

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
/// Currently holds only `files`; future keys (e.g. per-pattern exclude rules)
/// should be added here rather than inventing a parallel type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MagicConfig {
    /// Glob patterns for files to sync from main into worktrees.
    #[serde(default)]
    pub files: Vec<String>,
    // Future keys go here.
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

    Ok(Some(MagicConfig {
        files: merged_files,
    }))
}

/// Rewrite `.superset/magic.json` from `files`, pretty-printed with a
/// trailing newline.
// consumed by U9
#[allow(dead_code)]
pub fn write_magic_json(root: &Path, files: &[String]) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(MAGIC_JSON);
    let cfg = MagicConfig {
        files: files.to_vec(),
    };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg)?);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
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
