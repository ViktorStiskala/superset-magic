//! Read and write the `.superset/` workspace contract:
//!
//!   .superset/config.json         { setup, teardown, run }
//!   .superset/setup.sh            executable copy of the embedded asset
//!   .superset/setup_config.json   { files: [pattern, ...] }
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

const SUPERSET_DIR: &str = ".superset";
const CONFIG_JSON: &str = "config.json";
const SETUP_SH_NAME: &str = "setup.sh";
const SETUP_CONFIG_JSON: &str = "setup_config.json";

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
}
