//! Read and write the `.superset/` workspace contract:
//!
//!   .superset/config.json         { setup, teardown, run }
//!   .superset/setup.sh            executable copy of the embedded asset
//!   .superset/setup_config.json   { files: [pattern, ...] }
//!   .superset/magic.json          { files: [pattern, ...] }  (committed)
//!   .superset/magic.local.json    { files: [pattern, ...] }  (gitignored overlay)
//!
//! The embedded `setup.sh` (under `assets/`) is the single source of truth
//! for the script body; bootstrap mode overwrites the on-disk copy each
//! time. `config.json` is always rewritten in bootstrap mode from the
//! picker output; `teardown` and `run` arrays are preserved verbatim from
//! the existing on-disk `config.json` by merging upstream of the write.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Embedded canonical body of `setup.sh`. Bootstrap mode writes this
/// verbatim to `.superset/setup.sh`.
pub const SETUP_SH: &str = include_str!("../assets/setup.sh");

/// Embedded canonical body of `magic.sh`. Written to `.superset/magic.sh`
/// during migration (U9) and bootstrap. Delegates to `ss-magic` via `exec`
/// when the binary is available; otherwise prints an install hint and exits 0
/// so Superset's setup pipeline continues uninterrupted.
pub const MAGIC_SH: &str = include_str!("../assets/magic.sh");

const SUPERSET_DIR: &str = ".superset";
const CONFIG_JSON: &str = "config.json";
const SETUP_SH_NAME: &str = "setup.sh";
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


/// Shape of `.superset/setup_config.json`.
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

/// Snapshot of what's already present on disk. The bootstrap flow uses
/// this to decide between "first run" and "edit mode" banners and to
/// seed the multi-select with the existing patterns.
#[derive(Debug, Default)]
pub struct ExistingState {
    /// True when `.superset/` exists as a directory under the repo root.
    pub superset_dir_present: bool,
    /// Parsed `setup_config.json` content when present and well-formed.
    pub setup_config_json: Option<SetupConfig>,
    /// Parsed `config.json` content when present and well-formed. Bootstrap
    /// merges its `teardown` / `run` arrays into the new Config before
    /// writing.
    pub config_json: Option<Config>,
}

fn superset_dir(root: &Path) -> PathBuf {
    root.join(SUPERSET_DIR)
}

/// Read the current state of `.superset/` under `root` without writing
/// anything. Errors when `.superset/` exists as a non-directory, or when
/// either JSON file is malformed.
pub fn load_existing(root: &Path) -> Result<ExistingState> {
    let dir = superset_dir(root);
    if !dir.exists() {
        return Ok(ExistingState::default());
    }
    if !dir.is_dir() {
        bail!(
            "`{}` exists but is not a directory; remove or rename it before running bootstrap",
            dir.display()
        );
    }

    Ok(ExistingState {
        superset_dir_present: true,
        setup_config_json: load_setup_config(root)?,
        config_json: load_config(root)?,
    })
}

/// Load just `config.json` from `root/.superset/`. `Ok(None)` when the
/// file is absent; error when it exists but cannot be parsed.
pub fn load_config(root: &Path) -> Result<Option<Config>> {
    read_json::<Config>(&superset_dir(root).join(CONFIG_JSON)).with_context(|| {
        format!("reading {}", superset_dir(root).join(CONFIG_JSON).display())
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

/// Always overwrite `.superset/setup.sh` with the embedded canonical body
/// and mark it executable.
pub fn write_setup_sh(root: &Path) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(SETUP_SH_NAME);
    fs::write(&path, SETUP_SH).with_context(|| format!("writing {}", path.display()))?;
    chmod_executable(&path)?;
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

/// Rewrite `.superset/setup_config.json` from `files`, pretty-printed with
/// a trailing newline.
pub fn write_setup_config_json(root: &Path, files: &[String]) -> Result<()> {
    ensure_superset_dir(root)?;
    let path = superset_dir(root).join(SETUP_CONFIG_JSON);
    let cfg = SetupConfig {
        files: files.to_vec(),
    };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg)?);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// True when `.env` exists as a regular file at `root` and `.envrc` does
/// not. The orchestration uses this to decide whether to prompt the user
/// about creating `.envrc`.
pub fn should_offer_envrc(root: &Path) -> bool {
    root.join(".env").is_file() && !root.join(".envrc").exists()
}

/// Write `.envrc` at `root` with the body `dotenv_if_exists\n`.
pub fn write_envrc(root: &Path) -> Result<()> {
    let path = root.join(".envrc");
    fs::write(&path, "dotenv_if_exists\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Report of what `copy_into_repo` actually wrote at the real repo root.
#[derive(Debug, Default)]
pub struct MaterializeReport {
    /// True when `.envrc` was copied into the repo root (staging had one).
    pub wrote_envrc: bool,
}

/// Copy staged files from `stage_root` into `repo_root`, applying the
/// preservation rules:
///
/// - `.superset/setup.sh` is always overwritten and chmod 0755'd.
/// - `.superset/setup_config.json` is always overwritten.
/// - `.superset/config.json` is always overwritten. Preservation of any
///   existing `teardown` / `run` arrays must happen upstream by merging
///   into the staged Config before this call.
/// - `.envrc` is copied only when present at `stage_root`.
pub fn copy_into_repo(stage_root: &Path, repo_root: &Path) -> Result<MaterializeReport> {
    ensure_superset_dir(repo_root)?;
    let stage_dir = stage_root.join(SUPERSET_DIR);
    let real_dir = repo_root.join(SUPERSET_DIR);

    let stage_setup = stage_dir.join(SETUP_SH_NAME);
    let real_setup = real_dir.join(SETUP_SH_NAME);
    fs::copy(&stage_setup, &real_setup)
        .with_context(|| format!("copy {} → {}", stage_setup.display(), real_setup.display()))?;
    chmod_executable(&real_setup)?;

    let stage_setup_config = stage_dir.join(SETUP_CONFIG_JSON);
    let real_setup_config = real_dir.join(SETUP_CONFIG_JSON);
    fs::copy(&stage_setup_config, &real_setup_config).with_context(|| {
        format!(
            "copy {} → {}",
            stage_setup_config.display(),
            real_setup_config.display()
        )
    })?;

    let stage_config = stage_dir.join(CONFIG_JSON);
    let real_config = real_dir.join(CONFIG_JSON);
    fs::copy(&stage_config, &real_config).with_context(|| {
        format!(
            "copy {} → {}",
            stage_config.display(),
            real_config.display()
        )
    })?;

    let stage_envrc = stage_root.join(".envrc");
    let real_envrc = repo_root.join(".envrc");
    let wrote_envrc = if stage_envrc.exists() {
        fs::copy(&stage_envrc, &real_envrc).with_context(|| {
            format!("copy {} → {}", stage_envrc.display(), real_envrc.display())
        })?;
        true
    } else {
        false
    };

    Ok(MaterializeReport { wrote_envrc })
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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const OPTIONS: [&str; 4] = [".env", "**/.env", ".env.local", "**/.dev.vars"];

    fn fresh() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn cfg(setup: Vec<&str>, teardown: Vec<&str>, run: Vec<&str>) -> Config {
        Config {
            setup: setup.into_iter().map(String::from).collect(),
            teardown: teardown.into_iter().map(String::from).collect(),
            run: run.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn fresh_repo_emits_all_three_files() {
        let dir = fresh();
        let root = dir.path();
        write_setup_sh(root).unwrap();
        write_config_json(root, &cfg(vec!["./.superset/setup.sh"], vec![], vec![])).unwrap();
        write_setup_config_json(root, &[".env".to_string()]).unwrap();

        let dot = root.join(".superset");
        assert!(dot.join("setup.sh").is_file());
        assert!(dot.join("config.json").is_file());
        assert!(dot.join("setup_config.json").is_file());

        // setup.sh is executable on unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dot.join("setup.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        // setup.sh content matches the embedded asset
        let on_disk = fs::read_to_string(dot.join("setup.sh")).unwrap();
        assert_eq!(on_disk, SETUP_SH);

        // config.json matches the shape we wrote
        let parsed: Config =
            serde_json::from_str(&fs::read_to_string(dot.join("config.json")).unwrap()).unwrap();
        assert_eq!(parsed.setup, vec!["./.superset/setup.sh".to_string()]);
        assert!(parsed.teardown.is_empty());
        assert!(parsed.run.is_empty());
    }

    #[test]
    fn load_config_returns_none_when_absent() {
        let dir = fresh();
        assert!(load_config(dir.path()).unwrap().is_none());
    }

    #[test]
    fn load_config_round_trips() {
        let dir = fresh();
        let root = dir.path();
        fs::create_dir_all(root.join(".superset")).unwrap();
        let body = r#"{
          "setup": ["./.superset/setup.sh", "uv sync"],
          "teardown": ["./drop.sh"],
          "run": ["pnpm dev"]
        }"#;
        fs::write(root.join(".superset/config.json"), body).unwrap();

        let parsed = load_config(root).unwrap().unwrap();
        assert_eq!(parsed.setup, vec!["./.superset/setup.sh", "uv sync"]);
        assert_eq!(parsed.teardown, vec!["./drop.sh"]);
        assert_eq!(parsed.run, vec!["pnpm dev"]);
    }

    #[test]
    fn malformed_config_returns_clean_error() {
        let dir = fresh();
        let root = dir.path();
        fs::create_dir_all(root.join(".superset")).unwrap();
        fs::write(root.join(".superset/config.json"), "{not json").unwrap();
        let err = load_config(root).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("config.json"), "msg: {msg}");
        assert!(msg.contains("malformed JSON"), "msg: {msg}");
    }

    #[test]
    fn merge_setup_into_config_with_none_yields_empty_teardown_run() {
        let merged = merge_setup_into_config(None, vec!["./.superset/setup.sh".to_string()]);
        assert_eq!(merged.setup, vec!["./.superset/setup.sh".to_string()]);
        assert!(merged.teardown.is_empty());
        assert!(merged.run.is_empty());
    }

    #[test]
    fn merge_setup_into_config_preserves_teardown_and_run_verbatim() {
        let existing = cfg(
            vec!["./.superset/setup.sh"],
            vec!["./drop.sh", "psql -f cleanup.sql"],
            vec!["pnpm dev", "uv run task"],
        );
        let merged = merge_setup_into_config(
            Some(&existing),
            vec!["./.superset/setup.sh".into(), "uv sync".into()],
        );
        assert_eq!(merged.setup, vec!["./.superset/setup.sh", "uv sync"]);
        assert_eq!(merged.teardown, existing.teardown);
        assert_eq!(merged.run, existing.run);
    }

    #[test]
    fn write_config_json_is_pretty_with_trailing_newline_and_round_trips() {
        let dir = fresh();
        let root = dir.path();
        let original = cfg(
            vec!["./.superset/setup.sh", "uv sync"],
            vec!["./drop.sh"],
            vec!["pnpm dev"],
        );
        write_config_json(root, &original).unwrap();

        let raw = fs::read_to_string(root.join(".superset/config.json")).unwrap();
        assert!(raw.contains('\n'), "expected pretty-printed JSON");
        assert!(raw.ends_with('\n'), "expected trailing newline");

        let parsed = load_config(root).unwrap().unwrap();
        assert_eq!(parsed.setup, original.setup);
        assert_eq!(parsed.teardown, original.teardown);
        assert_eq!(parsed.run, original.run);
    }

    #[test]
    fn unknown_setup_config_entries_survive_rewrite() {
        let existing = vec![
            "apps/*/config".to_string(),
            ".env".to_string(),
            "packages/**/fixtures".to_string(),
        ];
        let unknown = existing_unknown_entries(&existing, &OPTIONS);
        assert_eq!(
            unknown,
            vec![
                "apps/*/config".to_string(),
                "packages/**/fixtures".to_string()
            ]
        );

        let dir = fresh();
        let root = dir.path();
        let selected = vec![".env".to_string(), "**/.dev.vars".to_string()];
        let mut merged = selected.clone();
        merged.extend(unknown);
        write_setup_config_json(root, &merged).unwrap();

        let parsed = load_setup_config(root).unwrap().unwrap();
        assert_eq!(
            parsed.files,
            vec![
                ".env".to_string(),
                "**/.dev.vars".to_string(),
                "apps/*/config".to_string(),
                "packages/**/fixtures".to_string(),
            ]
        );
    }

    #[test]
    fn malformed_setup_config_returns_clean_error() {
        let dir = fresh();
        let root = dir.path();
        fs::create_dir_all(root.join(".superset")).unwrap();
        fs::write(root.join(".superset/setup_config.json"), "{not json").unwrap();
        let err = load_setup_config(root).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("setup_config.json"), "msg: {msg}");
        assert!(msg.contains("malformed JSON"), "msg: {msg}");
    }

    #[test]
    fn write_envrc_writes_dotenv_if_exists() {
        let dir = fresh();
        let root = dir.path();
        write_envrc(root).unwrap();
        let body = fs::read_to_string(root.join(".envrc")).unwrap();
        assert_eq!(body, "dotenv_if_exists\n");
    }

    #[test]
    fn should_offer_envrc_true_when_env_alone() {
        let dir = fresh();
        let root = dir.path();
        fs::write(root.join(".env"), "FOO=1").unwrap();
        assert!(should_offer_envrc(root));
    }

    #[test]
    fn should_offer_envrc_false_when_envrc_exists() {
        let dir = fresh();
        let root = dir.path();
        fs::write(root.join(".env"), "FOO=1").unwrap();
        fs::write(root.join(".envrc"), "dotenv_if_exists\n").unwrap();
        assert!(!should_offer_envrc(root));
    }

    #[test]
    fn should_offer_envrc_false_when_env_absent() {
        let dir = fresh();
        let root = dir.path();
        assert!(!should_offer_envrc(root));
    }

    #[test]
    fn copy_into_repo_materializes_all_staged_files() {
        let stage = fresh();
        let dest = fresh();
        write_setup_sh(stage.path()).unwrap();
        write_config_json(
            stage.path(),
            &cfg(vec!["./.superset/setup.sh"], vec![], vec![]),
        )
        .unwrap();
        write_setup_config_json(stage.path(), &[".env".to_string()]).unwrap();
        write_envrc(stage.path()).unwrap();

        let report = copy_into_repo(stage.path(), dest.path()).unwrap();
        assert!(report.wrote_envrc);

        let real = dest.path().join(".superset");
        assert!(real.join("setup.sh").is_file());
        assert!(real.join("config.json").is_file());
        assert!(real.join("setup_config.json").is_file());
        assert!(dest.path().join(".envrc").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(real.join("setup.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn copy_into_repo_overwrites_existing_config_json() {
        let stage = fresh();
        let dest = fresh();
        write_setup_sh(stage.path()).unwrap();
        write_config_json(
            stage.path(),
            &cfg(vec!["./.superset/setup.sh", "uv sync"], vec![], vec![]),
        )
        .unwrap();
        write_setup_config_json(stage.path(), &[".env".to_string()]).unwrap();

        let dest_dir = dest.path().join(".superset");
        fs::create_dir_all(&dest_dir).unwrap();
        let pre_existing =
            r#"{"setup":["./.superset/setup.sh","./extra.sh"],"teardown":[],"run":[]}"#;
        fs::write(dest_dir.join("config.json"), pre_existing).unwrap();

        copy_into_repo(stage.path(), dest.path()).unwrap();
        let staged = fs::read_to_string(stage.path().join(".superset/config.json")).unwrap();
        let after = fs::read_to_string(dest_dir.join("config.json")).unwrap();
        assert_eq!(
            after, staged,
            "destination must mirror the staged config.json"
        );
    }

    #[test]
    fn bootstrap_simulation_preserves_teardown_across_rerun() {
        // Pre-existing config.json on disk carries a non-empty teardown.
        let dest = fresh();
        let dest_dir = dest.path().join(".superset");
        fs::create_dir_all(&dest_dir).unwrap();
        let pre_existing = r#"{"setup":["./old.sh"],"teardown":["./drop.sh"],"run":[]}"#;
        fs::write(dest_dir.join("config.json"), pre_existing).unwrap();

        // Bootstrap simulation: read existing, merge with new picker output, stage, copy.
        let existing = load_config(dest.path()).unwrap();
        let new_setup: Vec<String> = vec!["./.superset/setup.sh".into(), "uv sync".into()];
        let merged = merge_setup_into_config(existing.as_ref(), new_setup);

        let stage = fresh();
        write_setup_sh(stage.path()).unwrap();
        write_config_json(stage.path(), &merged).unwrap();
        write_setup_config_json(stage.path(), &[]).unwrap();

        copy_into_repo(stage.path(), dest.path()).unwrap();

        let final_cfg = load_config(dest.path()).unwrap().unwrap();
        assert_eq!(final_cfg.setup, vec!["./.superset/setup.sh", "uv sync"]);
        assert_eq!(final_cfg.teardown, vec!["./drop.sh".to_string()]);
        assert!(final_cfg.run.is_empty());
    }

    #[test]
    fn copy_into_repo_skips_envrc_when_not_staged() {
        let stage = fresh();
        let dest = fresh();
        write_setup_sh(stage.path()).unwrap();
        write_config_json(
            stage.path(),
            &cfg(vec!["./.superset/setup.sh"], vec![], vec![]),
        )
        .unwrap();
        write_setup_config_json(stage.path(), &[]).unwrap();
        // No write_envrc on stage.

        let report = copy_into_repo(stage.path(), dest.path()).unwrap();
        assert!(!report.wrote_envrc);
        assert!(!dest.path().join(".envrc").exists());
    }

    #[test]
    fn superset_as_file_returns_clear_error() {
        let dir = fresh();
        let root = dir.path();
        fs::write(root.join(".superset"), "not a dir").unwrap();
        let err = load_existing(root).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not a directory"), "msg: {msg}");
    }

    // ── MagicConfig / load_overlaid tests ────────────────────────────────────

    fn magic_dir(root: &std::path::Path) {
        fs::create_dir_all(root.join(".superset")).unwrap();
    }

    fn write_magic_json_raw(root: &std::path::Path, body: &str) {
        magic_dir(root);
        fs::write(root.join(".superset/magic.json"), body).unwrap();
    }

    fn write_magic_local_raw(root: &std::path::Path, body: &str) {
        magic_dir(root);
        fs::write(root.join(".superset/magic.local.json"), body).unwrap();
    }

    /// AE7 — union of distinct patterns; magic.json order first.
    #[test]
    fn ae7_overlay_unions_and_dedupes_files() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);
        write_magic_local_raw(root, r#"{"files":["**/.dev.vars"]}"#);

        let result = load_overlaid(root).unwrap().unwrap();
        assert_eq!(result.files, vec!["**/.env", "**/.dev.vars"]);
    }

    /// Local entry that repeats a base pattern appears only once (base position kept).
    #[test]
    fn overlay_dedupes_repeated_local_entry() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{"files":["**/.env","**/.dev.vars"]}"#);
        write_magic_local_raw(root, r#"{"files":["**/.dev.vars","extra.txt"]}"#);

        let result = load_overlaid(root).unwrap().unwrap();
        // **/.dev.vars must appear exactly once, in base position (index 1).
        assert_eq!(result.files, vec!["**/.env", "**/.dev.vars", "extra.txt"]);
    }

    /// magic.json present, magic.local.json absent → base only.
    #[test]
    fn overlay_base_only_when_local_absent() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{"files":["**/.env",".dev.vars"]}"#);

        let result = load_overlaid(root).unwrap().unwrap();
        assert_eq!(result.files, vec!["**/.env", ".dev.vars"]);
    }

    /// magic.json absent → Ok(None).
    #[test]
    fn overlay_returns_none_when_base_absent() {
        let dir = fresh();
        let root = dir.path();
        // No magic.json, not even a .superset dir.
        let result = load_overlaid(root).unwrap();
        assert!(result.is_none());
    }

    /// Malformed magic.json → error naming the path.
    #[test]
    fn overlay_malformed_base_returns_error_with_path() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, "{not json");

        let err = load_overlaid(root).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("magic.json"), "msg: {msg}");
        assert!(msg.contains("malformed JSON"), "msg: {msg}");
    }

    /// Malformed magic.local.json → error naming the path (no silent fallback).
    #[test]
    fn overlay_malformed_local_returns_error_with_path() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);
        write_magic_local_raw(root, "{bad json");

        let err = load_overlaid(root).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("magic.local.json"), "msg: {msg}");
        assert!(msg.contains("malformed JSON"), "msg: {msg}");
    }

    /// write_magic_json produces pretty-printed JSON with a trailing newline
    /// that round-trips through load_overlaid.
    #[test]
    fn write_magic_json_is_pretty_with_trailing_newline_and_round_trips() {
        let dir = fresh();
        let root = dir.path();
        let patterns = vec!["**/.env".to_string(), ".dev.vars".to_string()];
        write_magic_json(root, &patterns).unwrap();

        let raw = fs::read_to_string(root.join(".superset/magic.json")).unwrap();
        assert!(raw.contains('\n'), "expected pretty-printed JSON");
        assert!(raw.ends_with('\n'), "expected trailing newline");

        let result = load_overlaid(root).unwrap().unwrap();
        assert_eq!(result.files, patterns);
    }

    /// empty magic.json files array + non-empty local → local entries appended.
    #[test]
    fn overlay_empty_base_files_plus_local() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{"files":[]}"#);
        write_magic_local_raw(root, r#"{"files":["secrets/**"]}"#);

        let result = load_overlaid(root).unwrap().unwrap();
        assert_eq!(result.files, vec!["secrets/**"]);
    }

    /// Both magic.json and magic.local.json have no files key (serde default).
    #[test]
    fn overlay_missing_files_key_defaults_to_empty() {
        let dir = fresh();
        let root = dir.path();
        write_magic_json_raw(root, r#"{}"#);
        write_magic_local_raw(root, r#"{}"#);

        let result = load_overlaid(root).unwrap().unwrap();
        assert!(result.files.is_empty());
    }

    // ── bootstrap_magic_local_json / default_magic_files tests ───────────────

    /// Bootstrapped magic.local.json parses as {} (+ comment key) and overlays
    /// as empty files (the _comment key is ignored by serde).
    #[test]
    fn bootstrap_magic_local_json_creates_valid_overlay_noop() {
        let dir = fresh();
        let root = dir.path();

        // Need a magic.json so load_overlaid can return Some(_).
        write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);

        bootstrap_magic_local_json(root).unwrap();

        let path = root.join(".superset/magic.local.json");
        assert!(path.is_file(), "magic.local.json must be created");

        // Must be valid JSON.
        let raw = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .expect("bootstrapped magic.local.json must be valid JSON");
        assert!(parsed.is_object(), "must be a JSON object");
        assert!(parsed.get("_comment").is_some(), "must contain _comment key");

        // load_overlaid must round-trip: local contributes zero extra files.
        let result = load_overlaid(root).unwrap().unwrap();
        assert_eq!(
            result.files,
            vec!["**/.env"],
            "local overlay must add no files beyond the base"
        );
    }

    /// bootstrap_magic_local_json is idempotent: existing file is not overwritten.
    #[test]
    fn bootstrap_magic_local_json_idempotent_when_file_exists() {
        let dir = fresh();
        let root = dir.path();
        let path = root.join(".superset/magic.local.json");

        // Write a custom file first.
        fs::create_dir_all(root.join(".superset")).unwrap();
        let custom = r#"{"files":["custom/**"]}"#;
        fs::write(&path, custom).unwrap();

        bootstrap_magic_local_json(root).unwrap();

        // Must be unchanged.
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(after, custom, "existing file must not be overwritten");
    }

    /// Bootstrapped file has a trailing newline (consistent with the write convention).
    #[test]
    fn bootstrap_magic_local_json_has_trailing_newline() {
        let dir = fresh();
        let root = dir.path();

        bootstrap_magic_local_json(root).unwrap();

        let raw = fs::read_to_string(root.join(".superset/magic.local.json")).unwrap();
        assert!(raw.ends_with('\n'), "must end with a trailing newline");
    }

    /// default_magic_files includes .superset/magic.local.json.
    #[test]
    fn default_magic_files_includes_magic_local_json() {
        let defaults = default_magic_files();
        assert!(
            defaults.iter().any(|s| s == ".superset/magic.local.json"),
            "default_magic_files() must include .superset/magic.local.json; got: {defaults:?}"
        );
    }

    // ── write_magic_sh / magic.sh asset tests ────────────────────────────────

    /// write_magic_sh emits a file byte-equal to the embedded MAGIC_SH asset
    /// and marks it executable (mode 0755) on Unix.
    #[test]
    fn write_magic_sh_emits_executable_file_matching_embedded_asset() {
        let dir = fresh();
        let root = dir.path();
        write_magic_sh(root).unwrap();

        let path = root.join(".superset/magic.sh");
        assert!(path.is_file(), "magic.sh must be created");

        let on_disk = fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, MAGIC_SH, "on-disk content must match embedded asset");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "magic.sh must be mode 0755");
        }
    }

    /// Find bash via the host environment, bypassing any controlled PATH we set
    /// on child processes.  Returns the absolute path to bash, panicking if it
    /// cannot be located — the tests require bash.
    fn find_bash() -> std::path::PathBuf {
        // Try common locations so the test works regardless of the PATH value
        // we override on child processes.
        for candidate in &[
            "/opt/homebrew/bin/bash",
            "/usr/local/bin/bash",
            "/usr/bin/bash",
            "/bin/bash",
        ] {
            let p = std::path::Path::new(candidate);
            if p.exists() {
                return p.to_path_buf();
            }
        }
        // Fall back to whatever the host PATH exposes at test-compilation time.
        panic!("bash not found; tests require bash");
    }

    /// Covers AE8: running magic.sh with ss-magic absent from PATH prints an
    /// install error to stderr and exits 0 (pipeline must not be interrupted).
    #[test]
    fn ae8_magic_sh_absent_binary_prints_error_and_exits_zero() {
        let dir = fresh();
        let root = dir.path();
        write_magic_sh(root).unwrap();
        let script = root.join(".superset/magic.sh");

        // Use an empty temp dir as PATH so ss-magic is guaranteed absent.
        let empty_path_dir = tempfile::tempdir().unwrap();

        let output = std::process::Command::new(find_bash())
            .arg(&script)
            .env("PATH", empty_path_dir.path())
            // Ensure NO_COLOR is unset so the color branch is exercised (stderr
            // may or may not be a TTY in CI — we only verify the text content).
            .env_remove("NO_COLOR")
            .output()
            .expect("failed to run magic.sh via bash");

        assert_eq!(
            output.status.code(),
            Some(0),
            "magic.sh must exit 0 when ss-magic is absent; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("ss-magic is not installed"),
            "stderr must mention 'ss-magic is not installed'; got: {stderr}"
        );
        assert!(
            stderr.contains("ViktorStiskala/superset-magic"),
            "stderr must reference the install repo; got: {stderr}"
        );
    }

    /// Exit-code propagation via exec: a stub ss-magic that exits 3 must cause
    /// magic.sh to exit 3 as well.
    #[test]
    fn magic_sh_propagates_exit_code_from_ss_magic_via_exec() {
        let dir = fresh();
        let root = dir.path();
        write_magic_sh(root).unwrap();
        let script = root.join(".superset/magic.sh");

        // Create a stub ss-magic in a temp dir that always exits 3.
        let stub_dir = tempfile::tempdir().unwrap();
        let stub_path = stub_dir.path().join("ss-magic");
        fs::write(&stub_path, "#!/bin/sh\nexit 3\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&stub_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&stub_path, perms).unwrap();
        }

        // Prepend the stub dir to a minimal PATH so ss-magic resolves to our stub.
        let path_val = format!("{}:/usr/bin:/bin", stub_dir.path().display());

        let status = std::process::Command::new(find_bash())
            .arg(&script)
            .env("PATH", &path_val)
            .status()
            .expect("failed to run magic.sh via bash");

        assert_eq!(
            status.code(),
            Some(3),
            "magic.sh must propagate ss-magic's exit code (3) via exec"
        );
    }
}
