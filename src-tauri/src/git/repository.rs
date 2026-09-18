use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::platform::execution::ExecutionTarget;
use crate::platform::launchers::hide_console_window;
#[cfg(target_os = "windows")]
use crate::platform::wsl;

/// Shared with `wsl_git_arguments`, which embeds the same three names as
/// literal `env` argv elements, so the native and WSL paths cannot drift.
pub(crate) const GIT_ENV_VARS: [(&str, &str); 3] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_EDITOR", "true"),
    ("GIT_SEQUENCE_EDITOR", "true"),
];

fn native_git_command(root: &Path, args: &[&str]) -> Result<Command, String> {
    if which::which("git").is_err() {
        return Err(
            "Git wurde nicht gefunden. Installiere Git und starte Code Deck neu.".to_string(),
        );
    }
    let mut command = Command::new("git");
    command.args(args).current_dir(root);
    for (name, value) in GIT_ENV_VARS {
        command.env(name, value);
    }
    hide_console_window(&mut command);
    Ok(command)
}

#[cfg(target_os = "windows")]
fn wsl_git_command(distro: &str, linux_path: &str, args: &[&str]) -> Result<Command, String> {
    let distro = distro.trim();
    let linux_path = linux_path.trim();
    if distro.is_empty() || linux_path.is_empty() {
        let detail = if distro.is_empty() {
            "Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus."
        } else {
            "Für dieses Projekt ist kein Linux-Pfad hinterlegt. Lege ihn in den Projekteinstellungen fest."
        };
        return Err(wsl::wsl_error_message(
            "Git konnte nicht ausgeführt werden.",
            distro,
            linux_path,
            detail,
        ));
    }
    let mut command = Command::new("wsl.exe");
    command.args(wsl::wsl_git_arguments(
        distro,
        linux_path,
        &GIT_ENV_VARS,
        args,
    ));
    hide_console_window(&mut command);
    Ok(command)
}

#[cfg(not(target_os = "windows"))]
fn wsl_git_command(_distro: &str, _linux_path: &str, _args: &[&str]) -> Result<Command, String> {
    Err("WSL ist nur unter Windows verfügbar.".to_string())
}

// Native's working directory is `root`; WSL's is the target's own
// `linux_path` via `--cd`, so `root` (which may be a host UNC path) is never
// passed to it.
fn git_command(target: &ExecutionTarget, root: &Path, args: &[&str]) -> Result<Command, String> {
    match target {
        ExecutionTarget::Native { .. } => native_git_command(root, args),
        ExecutionTarget::Wsl { distro, linux_path } => wsl_git_command(distro, linux_path, args),
    }
}

#[cfg(target_os = "windows")]
fn decode_git_output(target: &ExecutionTarget, bytes: &[u8]) -> String {
    if matches!(target, ExecutionTarget::Wsl { .. }) {
        // git inside WSL emits UTF-8, but wsl.exe's own failures are
        // UTF-16LE, so the mixed decoder is needed only on this path.
        wsl::decode_wsl_output(bytes).trim().to_string()
    } else {
        String::from_utf8_lossy(bytes).trim().to_string()
    }
}

#[cfg(not(target_os = "windows"))]
fn decode_git_output(_target: &ExecutionTarget, bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

#[cfg(target_os = "windows")]
fn wsl_distro_error(
    target: &ExecutionTarget,
    exit_code: Option<i32>,
    stderr: &str,
) -> Option<String> {
    let ExecutionTarget::Wsl { distro, linux_path } = target else {
        return None;
    };
    wsl::is_distro_not_found(exit_code, stderr).then(|| {
        wsl::wsl_error_message(
            "Git konnte nicht ausgeführt werden.",
            distro,
            linux_path,
            "Die ausgewählte WSL-Distribution ist nicht verfügbar.",
        )
    })
}

#[cfg(not(target_os = "windows"))]
fn wsl_distro_error(
    _target: &ExecutionTarget,
    _exit_code: Option<i32>,
    _stderr: &str,
) -> Option<String> {
    None
}

pub(crate) fn git_output(target: &ExecutionTarget, root: &Path, args: &[&str]) -> Option<String> {
    let mut command = git_command(target, root, args).ok()?;
    command.stdin(Stdio::null()).stderr(Stdio::null());
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| decode_git_output(target, &output.stdout))
}

pub(crate) fn run_git(
    target: &ExecutionTarget,
    root: &Path,
    args: &[String],
) -> Result<String, String> {
    let string_args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = git_command(target, root, &string_args)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .map_err(|error| format!("Git konnte nicht gestartet werden: {error}"))?;

    let stdout = decode_git_output(target, &output.stdout);
    let stderr = decode_git_output(target, &output.stderr);

    if output.status.success() {
        return Ok(if stdout.is_empty() { stderr } else { stdout });
    }

    if let Some(message) = wsl_distro_error(target, output.status.code(), &stderr) {
        return Err(message);
    }

    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("Git wurde mit Status {} beendet.", output.status)
    })
}

pub(crate) fn git_project_root(
    target: &ExecutionTarget,
    project_path: &str,
) -> Result<PathBuf, String> {
    let root = PathBuf::from(project_path);
    if !root.is_dir() {
        return Err(format!("Projektordner nicht gefunden: {project_path}"));
    }
    let is_git = git_output(target, &root, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == "true");
    if !is_git {
        return Err("Der Projektordner ist kein Git-Repository.".to_string());
    }
    Ok(root)
}

#[cfg(target_os = "windows")]
fn wsl_path_exists(distro: &str, linux_path: &str, git_path: &str) -> bool {
    // `--git-path` answers relative to the working directory, and that
    // directory is the Linux one under WSL git, so existence has to be
    // checked inside the distro rather than with `std::fs` on the host.
    let path = if git_path.starts_with('/') {
        git_path.to_string()
    } else {
        wsl::join_linux_path(linux_path, git_path)
    };
    let mut command = Command::new("wsl.exe");
    command
        .args(wsl::wsl_test_path_arguments(distro, &path))
        .stdin(Stdio::null());
    hide_console_window(&mut command);
    command.status().is_ok_and(|status| status.success())
}

#[cfg(not(target_os = "windows"))]
fn wsl_path_exists(_distro: &str, _linux_path: &str, _git_path: &str) -> bool {
    false
}

fn git_path_exists(target: &ExecutionTarget, root: &Path, name: &str) -> bool {
    let Some(value) = git_output(target, root, &["rev-parse", "--git-path", name]) else {
        return false;
    };
    match target {
        ExecutionTarget::Native { .. } => {
            let path = PathBuf::from(&value);
            if path.is_absolute() {
                path.exists()
            } else {
                root.join(path).exists()
            }
        }
        ExecutionTarget::Wsl { distro, linux_path } => wsl_path_exists(distro, linux_path, &value),
    }
}

pub(crate) fn current_git_operation(target: &ExecutionTarget, root: &Path) -> Option<String> {
    if git_path_exists(target, root, "MERGE_HEAD") {
        Some("merge".to_string())
    } else if git_path_exists(target, root, "rebase-merge")
        || git_path_exists(target, root, "rebase-apply")
    {
        Some("rebase".to_string())
    } else if git_path_exists(target, root, "CHERRY_PICK_HEAD") {
        Some("cherry-pick".to_string())
    } else if git_path_exists(target, root, "REVERT_HEAD") {
        Some("revert".to_string())
    } else {
        None
    }
}

pub(crate) fn safe_repo_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Ungültiger Repository-Pfad.".to_string());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Projektpfad konnte nicht geprüft werden: {error}"))?;
    let target = canonical_root.join(relative_path);
    let mut existing_ancestor = target.parent().unwrap_or(&canonical_root);
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| "Dateipfad konnte nicht geprüft werden.".to_string())?;
    }
    let canonical_ancestor = existing_ancestor
        .canonicalize()
        .map_err(|error| format!("Dateipfad konnte nicht geprüft werden: {error}"))?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err("Der Dateipfad liegt außerhalb des Projekts.".to_string());
    }
    if target.exists() {
        let canonical_target = target
            .canonicalize()
            .map_err(|error| format!("Dateipfad konnte nicht geprüft werden: {error}"))?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err("Die Datei verweist auf einen Pfad außerhalb des Projekts.".to_string());
        }
    }
    Ok(target)
}

pub(crate) fn read_git_stage(
    target: &ExecutionTarget,
    root: &Path,
    stage: u8,
    file_path: &str,
) -> Result<(Option<String>, bool), String> {
    let spec = format!(":{stage}:{file_path}");
    let mut command = git_command(target, root, &["show", &spec])?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = command
        .output()
        .map_err(|error| format!("Konfliktversion konnte nicht gelesen werden: {error}"))?;
    if !output.status.success() {
        return Ok((None, false));
    }
    match String::from_utf8(output.stdout) {
        Ok(value) => Ok((Some(value), false)),
        Err(_) => Ok((None, true)),
    }
}
