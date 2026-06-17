//! Detect which preconfigured setup commands should be preselected in the
//! bootstrap picker, driven by root lockfiles and the root `package.json`
//! scripts map.
//!
//! Mirrors `repo_scan`'s shape: a fixed `OPTIONS` array in display order
//! and a `detect_for_options(root) -> Vec<bool>` aligned to it. Permission
//! errors and missing/unparseable files for the optional script signal are
//! silent — they collapse to "no signal" rather than aborting the scan.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

/// The preconfigured command rows offered in bootstrap mode, in display
/// order. Order is load-bearing — `detect_for_options` returns a bool
/// vector aligned to this array.
pub const OPTIONS: [&str; 6] = [
    "./.superset/setup.sh",
    "pnpm -r install",
    "pnpm -r run cf-typegen",
    "npm ci",
    "yarn install --frozen-lockfile",
    "uv sync",
];

/// npm-script names that, when present in the root `package.json` `scripts`
/// map, are strong enough signals to preselect a `<pm> run <name>` row.
///
/// Deliberately conservative for v1: `setup` and `codegen` are excluded
/// because both are among the most-recycled npm-script slot names — a
/// `setup` script may install husky hooks and a `codegen` script may take
/// minutes and hit remote services. Preselecting either by name alone
/// would be a strong endorsement this layer cannot honestly make. Users
/// add anything not in this list via the picker's "+ Add new command…"
/// path.
const RECOGNIZED_SCRIPTS: [&str; 1] = ["cf-typegen"];

/// JS package manager inferred from root lockfiles. `Pnpm` wins when
/// multiple lockfiles coexist; the rationale is that `pnpm-lock.yaml` in a
/// repo that also has `package-lock.json` is almost always the
/// authoritative lockfile in this org's monorepos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Pnpm,
    Npm,
    Yarn,
    None,
}

fn detect_package_manager(root: &Path) -> PackageManager {
    if root.join("pnpm-lock.yaml").is_file() {
        PackageManager::Pnpm
    } else if root.join("package-lock.json").is_file() {
        PackageManager::Npm
    } else if root.join("yarn.lock").is_file() {
        PackageManager::Yarn
    } else {
        PackageManager::None
    }
}

#[derive(Debug, Deserialize)]
struct PackageJsonShape {
    #[serde(default)]
    scripts: std::collections::HashMap<String, String>,
}

/// Read `<root>/package.json` and return the keys of its `scripts` object.
/// Returns an empty set when the file is absent, unparseable, has no
/// `scripts` field, or is unreadable due to permissions — script signal is
/// optional, so failure is silent.
fn parse_root_package_scripts(root: &Path) -> HashSet<String> {
    let path = root.join("package.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return HashSet::new(),
    };
    match serde_json::from_str::<PackageJsonShape>(&raw) {
        Ok(pkg) => pkg.scripts.into_keys().collect(),
        Err(_) => HashSet::new(),
    }
}

/// For each option in [`OPTIONS`], `true` when the option should be
/// preselected in the bootstrap picker based on cheap repo signals.
/// Returns a vector aligned to `OPTIONS`.
pub fn detect_for_options(root: &Path) -> Result<Vec<bool>> {
    let pm = detect_package_manager(root);
    let scripts = parse_root_package_scripts(root);
    let uv_present = root.join("uv.lock").is_file();

    let detected = OPTIONS
        .iter()
        .map(|opt| match *opt {
            "./.superset/setup.sh" => true,
            "pnpm -r install" => pm == PackageManager::Pnpm,
            "pnpm -r run cf-typegen" => {
                pm == PackageManager::Pnpm
                    && RECOGNIZED_SCRIPTS.iter().any(|s| scripts.contains(*s))
            }
            "npm ci" => pm == PackageManager::Npm,
            "yarn install --frozen-lockfile" => pm == PackageManager::Yarn,
            "uv sync" => uv_present,
            _ => false,
        })
        .collect();
    Ok(detected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// Index helpers to keep tests readable when OPTIONS reorders.
    fn idx(opt: &str) -> usize {
        OPTIONS.iter().position(|o| *o == opt).unwrap()
    }

    #[test]
    fn empty_repo_only_setup_sh_detected() {
        let dir = fresh();
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("./.superset/setup.sh")]);
        assert!(!hits[idx("pnpm -r install")]);
        assert!(!hits[idx("pnpm -r run cf-typegen")]);
        assert!(!hits[idx("npm ci")]);
        assert!(!hits[idx("yarn install --frozen-lockfile")]);
        assert!(!hits[idx("uv sync")]);
    }

    #[test]
    fn pnpm_lockfile_alone_picks_pnpm_install() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("pnpm -r install")]);
        assert!(!hits[idx("npm ci")]);
        assert!(!hits[idx("yarn install --frozen-lockfile")]);
    }

    #[test]
    fn npm_lockfile_alone_picks_npm_ci() {
        let dir = fresh();
        touch(dir.path(), "package-lock.json");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("npm ci")]);
        assert!(!hits[idx("pnpm -r install")]);
        assert!(!hits[idx("yarn install --frozen-lockfile")]);
    }

    #[test]
    fn yarn_lockfile_alone_picks_yarn_install() {
        let dir = fresh();
        touch(dir.path(), "yarn.lock");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("yarn install --frozen-lockfile")]);
        assert!(!hits[idx("pnpm -r install")]);
        assert!(!hits[idx("npm ci")]);
    }

    #[test]
    fn pnpm_wins_when_pnpm_and_npm_coexist() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        touch(dir.path(), "package-lock.json");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("pnpm -r install")]);
        assert!(!hits[idx("npm ci")]);
    }

    #[test]
    fn uv_lock_detected_independently_of_js() {
        let dir = fresh();
        touch(dir.path(), "uv.lock");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("uv sync")]);
        assert!(!hits[idx("pnpm -r install")]);
    }

    #[test]
    fn cf_typegen_script_plus_pnpm_picks_flavored_row() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"cf-typegen":"wrangler types"}}"#,
        );
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(hits[idx("pnpm -r run cf-typegen")]);
        assert!(hits[idx("pnpm -r install")]);
    }

    #[test]
    fn package_json_with_no_scripts_key_is_silent() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        write(dir.path(), "package.json", r#"{"name":"x"}"#);
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(!hits[idx("pnpm -r run cf-typegen")]);
        assert!(hits[idx("pnpm -r install")]);
    }

    #[test]
    fn unparseable_package_json_is_silent() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        write(dir.path(), "package.json", "{not json");
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(!hits[idx("pnpm -r run cf-typegen")]);
        assert!(hits[idx("pnpm -r install")]);
    }

    #[test]
    fn nested_package_json_is_ignored() {
        let dir = fresh();
        touch(dir.path(), "pnpm-lock.yaml");
        write(
            dir.path(),
            "apps/api/package.json",
            r#"{"scripts":{"cf-typegen":"wrangler types"}}"#,
        );
        let hits = detect_for_options(dir.path()).unwrap();
        assert!(!hits[idx("pnpm -r run cf-typegen")]);
    }

    #[test]
    fn cf_typegen_under_npm_does_not_pick_pnpm_row() {
        let dir = fresh();
        touch(dir.path(), "package-lock.json");
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"cf-typegen":"wrangler types"}}"#,
        );
        let hits = detect_for_options(dir.path()).unwrap();
        // Script-flavored preselect is pnpm-only for v1 (see plan).
        assert!(!hits[idx("pnpm -r run cf-typegen")]);
        assert!(hits[idx("npm ci")]);
    }
}
