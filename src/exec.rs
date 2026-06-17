//! Execute setup commands from `.superset/config.json` after apply-mode
//! file copy completes. Mirrors upstream Superset's execution semantics:
//! commands joined with ` && ` into one `$SHELL -lc` invocation so the
//! user's shell rc (nvm, pnpm, asdf shims) is on PATH; working directory
//! is the apply destination (the worktree); two `SUPERSET_*` env vars
//! exposed.
//!
//! `$SHELL` falls back to `/bin/sh` with the `-c` flag (no `-l`) when
//! unset, because POSIX `/bin/sh` (dash on Linux) does not support `-l`.

use std::path::Path;
use std::process::{Command, ExitStatus};

use anyhow::{bail, Context, Result};

/// Per-invocation event emitted while running setup commands.
#[derive(Debug, Clone)]
pub enum Event {
    /// Execution is starting. `display` is the human-readable invocation
    /// (e.g., `/bin/zsh -lc "pnpm install && uv sync"` or `bash /path/to/setup.sh`).
    Begin { display: String },
    /// The child has exited.
    Complete { status: ExitStatus },
}

/// Render an `ExitStatus` for display. Signal-killed children have no
/// numeric exit code; render those as `"signal"` so error messages stay
/// readable.
pub fn format_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(n) => n.to_string(),
        None => "signal".to_string(),
    }
}

/// Human-readable preview of the shell invocation that [`run`] will use
/// for the given `commands`. `apply_flow` prints this above the
/// confirm-before-run prompt so the user sees exactly what will execute.
pub fn invocation_preview(commands: &[String]) -> String {
    let (shell, flag) = resolve_shell();
    format!("{} {} \"{}\"", shell, flag, commands.join(" && "))
}

/// Run `commands` as one ` && `-joined shell invocation.
///
/// `$SHELL -lc` is used when `$SHELL` is set (matches upstream's
/// PTY-in-user-shell semantic so nvm/pnpm/asdf shims load from rc files).
/// Falls back to `/bin/sh -c` (no `-l`) when `$SHELL` is unset, because
/// POSIX `/bin/sh` does not support `-l`.
///
/// Two env vars are exposed to the child:
/// - `SUPERSET_ROOT_PATH` — absolute path to the main checkout
/// - `SUPERSET_WORKSPACE_PATH` — absolute path to the worktree
///
/// `commands` should never be empty — callers route empty arrays to
/// [`run_setup_sh`] before reaching this function.
pub fn run<F>(
    workspace_root: &Path,
    main_root: &Path,
    commands: &[String],
    on_event: F,
) -> Result<ExitStatus>
where
    F: FnMut(&Event),
{
    let (shell, flag) = resolve_shell();
    run_with_shell(&shell, flag, workspace_root, main_root, commands, on_event)
}

/// Run `setup_sh` directly via `bash <path>` (no shell wrapping). Used by
/// `apply_flow` for the empty-array fallback. Avoids `sh -c` quoting bugs
/// when paths contain spaces or shell metacharacters.
pub fn run_setup_sh<F>(
    workspace_root: &Path,
    main_root: &Path,
    setup_sh: &Path,
    mut on_event: F,
) -> Result<ExitStatus>
where
    F: FnMut(&Event),
{
    ensure_workspace_root(workspace_root)?;
    let display = format!("bash {}", setup_sh.display());
    on_event(&Event::Begin {
        display: display.clone(),
    });
    let status = Command::new("bash")
        .arg(setup_sh)
        .current_dir(workspace_root)
        .env("SUPERSET_ROOT_PATH", main_root)
        .env("SUPERSET_WORKSPACE_PATH", workspace_root)
        .status()
        .with_context(|| format!("invoking {display}"))?;
    on_event(&Event::Complete { status });
    Ok(status)
}

fn resolve_shell() -> (String, &'static str) {
    match std::env::var("SHELL") {
        Ok(s) => (s, "-lc"),
        Err(_) => ("/bin/sh".to_string(), "-c"),
    }
}

fn run_with_shell<F>(
    shell: &str,
    flag: &str,
    workspace_root: &Path,
    main_root: &Path,
    commands: &[String],
    mut on_event: F,
) -> Result<ExitStatus>
where
    F: FnMut(&Event),
{
    ensure_workspace_root(workspace_root)?;
    let joined = commands.join(" && ");
    let display = format!("{} {} \"{}\"", shell, flag, joined);
    on_event(&Event::Begin {
        display: display.clone(),
    });
    let status = Command::new(shell)
        .arg(flag)
        .arg(&joined)
        .current_dir(workspace_root)
        .env("SUPERSET_ROOT_PATH", main_root)
        .env("SUPERSET_WORKSPACE_PATH", workspace_root)
        .status()
        .with_context(|| format!("invoking {display}"))?;
    on_event(&Event::Complete { status });
    Ok(status)
}

fn ensure_workspace_root(workspace_root: &Path) -> Result<()> {
    if !workspace_root.exists() {
        bail!(
            "workspace root {} does not exist",
            workspace_root.display()
        );
    }
    if !workspace_root.is_dir() {
        bail!(
            "workspace root {} is not a directory",
            workspace_root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn fresh() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Force a deterministic shell + flag in tests so behaviour does not
    /// depend on the caller's `$SHELL` (which varies across CI / dev).
    fn run_in(
        ws: &Path,
        main: &Path,
        commands: &[String],
        events: &mut Vec<Event>,
    ) -> Result<ExitStatus> {
        run_with_shell("/bin/sh", "-c", ws, main, commands, |ev| events.push(ev.clone()))
    }

    #[test]
    fn happy_path_two_commands_short_circuit_via_and() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        let status = run_in(
            ws.path(),
            main.path(),
            &["touch a".into(), "touch b".into()],
            &mut events,
        )
        .unwrap();
        assert!(status.success());
        assert!(ws.path().join("a").exists());
        assert!(ws.path().join("b").exists());
        assert_eq!(events.len(), 2);
        match &events[0] {
            Event::Begin { display } => {
                assert!(display.contains("touch a && touch b"), "display: {display}")
            }
            _ => panic!("expected Begin"),
        }
        match &events[1] {
            Event::Complete { status } => assert!(status.success()),
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn single_command_no_and_join_artifact() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        run_in(ws.path(), main.path(), &["touch single".into()], &mut events).unwrap();
        assert!(ws.path().join("single").is_file());
        match &events[0] {
            Event::Begin { display } => {
                assert!(display.contains("\"touch single\""), "display: {display}");
                assert!(!display.contains("&&"), "single-cmd display: {display}");
            }
            _ => panic!("expected Begin"),
        }
    }

    #[test]
    fn false_exits_non_zero() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        let status = run_in(ws.path(), main.path(), &["false".into()], &mut events).unwrap();
        assert!(!status.success());
    }

    #[test]
    fn short_circuit_skips_post_failure_commands() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        let status = run_in(
            ws.path(),
            main.path(),
            &["touch a".into(), "false".into(), "touch b".into()],
            &mut events,
        )
        .unwrap();
        assert!(!status.success());
        assert!(ws.path().join("a").exists());
        assert!(!ws.path().join("b").exists());
    }

    #[test]
    fn cd_shares_shell_state_within_joined_invocation() {
        let ws = fresh();
        let main = fresh();
        fs::create_dir(ws.path().join("subdir")).unwrap();
        let mut events = Vec::new();
        run_in(
            ws.path(),
            main.path(),
            &["cd subdir && touch x".into()],
            &mut events,
        )
        .unwrap();
        assert!(ws.path().join("subdir/x").exists());
    }

    #[test]
    fn env_vars_visible_to_child() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        run_in(
            ws.path(),
            main.path(),
            &[
                r#"printf %s "$SUPERSET_ROOT_PATH" > out_root"#.into(),
                r#"printf %s "$SUPERSET_WORKSPACE_PATH" > out_ws"#.into(),
            ],
            &mut events,
        )
        .unwrap();
        let root_contents = fs::read_to_string(ws.path().join("out_root")).unwrap();
        let ws_contents = fs::read_to_string(ws.path().join("out_ws")).unwrap();
        assert_eq!(root_contents, main.path().display().to_string());
        assert_eq!(ws_contents, ws.path().display().to_string());
    }

    #[test]
    fn workspace_name_env_var_is_not_set() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        run_in(
            ws.path(),
            main.path(),
            // ${VAR:+set} prints "set" if VAR is non-empty, else empty.
            &[r#"printf %s "${SUPERSET_WORKSPACE_NAME:+set}" > out"#.into()],
            &mut events,
        )
        .unwrap();
        let contents = fs::read_to_string(ws.path().join("out")).unwrap();
        assert_eq!(contents, "", "SUPERSET_WORKSPACE_NAME must not be set");
    }

    #[test]
    fn nonexistent_workspace_root_errors() {
        let main = fresh();
        let nonexistent = PathBuf::from("/this/path/does/not/exist/superset-setup-test");
        let mut events = Vec::new();
        let err = run_in(&nonexistent, main.path(), &["true".into()], &mut events).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("does not exist"), "msg: {msg}");
        assert!(events.is_empty(), "no events should fire");
    }

    #[test]
    fn run_setup_sh_with_path_containing_space() {
        let ws = fresh();
        let main = fresh();
        let main_with_space = main.path().join("My Repo");
        fs::create_dir_all(main_with_space.join(".superset")).unwrap();
        let setup_sh = main_with_space.join(".superset/setup.sh");
        fs::write(&setup_sh, "#!/usr/bin/env bash\ntouch ran-via-fallback\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&setup_sh).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&setup_sh, perms).unwrap();
        }

        let mut events = Vec::new();
        let status = run_setup_sh(ws.path(), &main_with_space, &setup_sh, |ev| {
            events.push(ev.clone())
        })
        .unwrap();
        assert!(status.success());
        assert!(ws.path().join("ran-via-fallback").exists());
        match &events[0] {
            Event::Begin { display } => {
                assert!(display.starts_with("bash "), "display: {display}");
                assert!(display.contains("My Repo"), "display: {display}");
            }
            _ => panic!("expected Begin"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_signal_termination() {
        use std::os::unix::process::ExitStatusExt;
        // Raw value 9 encodes "killed by signal 9" (low byte is the signal,
        // no exit code byte set). status.code() returns None for this.
        let status = ExitStatus::from_raw(9);
        assert!(status.code().is_none());
        assert_eq!(format_exit(status), "signal");
    }

    #[test]
    fn format_exit_normal_termination() {
        let ws = fresh();
        let main = fresh();
        let mut events = Vec::new();
        let status = run_in(ws.path(), main.path(), &["true".into()], &mut events).unwrap();
        assert_eq!(format_exit(status), "0");
    }
}
