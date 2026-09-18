use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "windows")]
use crate::platform::launchers::hide_console_window;
use crate::platform::launchers::shell_command;
#[cfg(target_os = "windows")]
use crate::platform::wsl;
#[cfg(target_os = "windows")]
use crate::projects::validation::display_path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NativeShell {
    #[default]
    PlatformDefault,
    Cmd,
    PowerShell7,
    WindowsPowerShell,
}

#[cfg(target_os = "windows")]
impl NativeShell {
    fn german_display_name(self) -> &'static str {
        match self {
            NativeShell::PlatformDefault => "Plattform-Standard",
            NativeShell::Cmd => "Eingabeaufforderung",
            NativeShell::PowerShell7 => "PowerShell 7",
            NativeShell::WindowsPowerShell => "Windows PowerShell",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ExecutionTarget {
    Native {
        #[serde(default)]
        shell: NativeShell,
    },
    #[serde(rename_all = "camelCase")]
    Wsl { distro: String, linux_path: String },
}

impl Default for ExecutionTarget {
    fn default() -> Self {
        ExecutionTarget::Native {
            shell: NativeShell::PlatformDefault,
        }
    }
}

// `chcp` is an external process: if it ever fails without touching stdout,
// it leaves $LASTEXITCODE set, and a later succeeding cmdlet does not clear
// it — poisoning our own exit-code suffix. This form sets the console
// encoding in-process and resets $LASTEXITCODE explicitly.
#[cfg(target_os = "windows")]
const POWERSHELL_UTF8_PREFIX: &str =
    "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$LASTEXITCODE = 0";

// pwsh -Command collapses any failing native command's exit code to 1.
// $LASTEXITCODE holds the real one; a non-terminating cmdlet failure instead
// leaves $LASTEXITCODE untouched but clears $?, so both are checked.
#[cfg(target_os = "windows")]
const POWERSHELL_EXIT_CODE_SUFFIX: &str =
    "if (-not $?) { exit $(if ($LASTEXITCODE) { $LASTEXITCODE } else { 1 }) }\nexit $LASTEXITCODE";

#[cfg(target_os = "windows")]
pub(crate) fn windows_shell_executable(shell: NativeShell) -> &'static str {
    match shell {
        NativeShell::PlatformDefault | NativeShell::Cmd => "cmd.exe",
        NativeShell::PowerShell7 => "pwsh.exe",
        NativeShell::WindowsPowerShell => "powershell.exe",
    }
}

// Newline-joined, not `;`: a script ending in a `#` comment would swallow a
// `;`-joined suffix and the exit-code lines would never run.
#[cfg(target_os = "windows")]
fn powershell_script(script: &str) -> String {
    format!("{POWERSHELL_UTF8_PREFIX}\n{script}\n{POWERSHELL_EXIT_CODE_SUFFIX}")
}

#[cfg(target_os = "windows")]
fn powershell_arguments(script: &str) -> Vec<String> {
    vec![
        "-NoLogo".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        powershell_script(script),
    ]
}

#[cfg(target_os = "windows")]
pub(crate) fn resolve_windows_shell(shell: NativeShell) -> Option<PathBuf> {
    match shell {
        NativeShell::PlatformDefault | NativeShell::Cmd => Some(PathBuf::from("cmd.exe")),
        NativeShell::PowerShell7 => {
            if let Ok(path) = which::which("pwsh") {
                return Some(path);
            }
            let mut candidates = Vec::new();
            if let Some(root) = std::env::var_os("ProgramFiles") {
                candidates.push(PathBuf::from(root).join(r"PowerShell\7\pwsh.exe"));
            }
            if let Some(root) = std::env::var_os("ProgramFiles(x86)") {
                candidates.push(PathBuf::from(root).join(r"PowerShell\7\pwsh.exe"));
            }
            if let Some(root) = std::env::var_os("LOCALAPPDATA") {
                candidates.push(PathBuf::from(root).join(r"Microsoft\WindowsApps\pwsh.exe"));
            }
            candidates.into_iter().find(|candidate| candidate.is_file())
        }
        NativeShell::WindowsPowerShell => {
            if let Some(root) = std::env::var_os("SystemRoot") {
                let candidate =
                    PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            which::which("powershell").ok()
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn shell_not_found_message(
    headline: &str,
    shell: NativeShell,
    executable: &str,
    working_dir: &Path,
) -> String {
    format!(
        "{headline}\n\nUmgebung: Windows\nShell: {}\nProgramm: {executable}\nProjekt: {}\n\n{executable} wurde nicht gefunden. Wähle in den Einstellungen eine andere Command-Shell.",
        shell.german_display_name(),
        display_path(working_dir)
    )
}

#[cfg(target_os = "windows")]
fn build_windows_command(
    target: &ExecutionTarget,
    script: &str,
    working_dir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Command, String> {
    match target {
        ExecutionTarget::Native { shell } => {
            build_windows_native_command(*shell, script, working_dir, env)
        }
        ExecutionTarget::Wsl { distro, linux_path } => {
            build_wsl_command(distro, linux_path, script, env)
        }
    }
}

#[cfg(target_os = "windows")]
fn build_windows_native_command(
    shell: NativeShell,
    script: &str,
    working_dir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Command, String> {
    if matches!(shell, NativeShell::PlatformDefault | NativeShell::Cmd) {
        let mut command = shell_command(script);
        command.current_dir(working_dir).envs(env);
        return Ok(command);
    }

    let executable = windows_shell_executable(shell);
    let program = resolve_windows_shell(shell).ok_or_else(|| {
        shell_not_found_message(
            "Command konnte nicht gestartet werden.",
            shell,
            executable,
            working_dir,
        )
    })?;

    let mut command = Command::new(&program);
    command
        .args(powershell_arguments(script))
        .current_dir(working_dir)
        // Defaults first, `env` last: a user-configured PYTHONUTF8 must win,
        // matching the pre-PR order where `.envs()` was applied after this
        // command was built.
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .envs(env);
    hide_console_window(&mut command);
    Ok(command)
}

#[cfg(target_os = "windows")]
fn build_wsl_command(
    distro: &str,
    linux_path: &str,
    script: &str,
    env: &BTreeMap<String, String>,
) -> Result<Command, String> {
    let distro = distro.trim();
    let linux_path = linux_path.trim();
    if distro.is_empty() || linux_path.is_empty() {
        let detail = if distro.is_empty() {
            "Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus."
        } else {
            "Für dieses Projekt ist kein Linux-Pfad hinterlegt. Lege ihn in den Projekteinstellungen fest."
        };
        return Err(wsl::wsl_error_message(
            "Command konnte nicht gestartet werden.",
            distro,
            linux_path,
            detail,
        ));
    }

    // Unlike native, an invalid name here could corrupt the `WSLENV` list on
    // the Linux side, so it is rejected instead of passed through.
    let env = wsl::validated_env(env)?;

    let mut command = Command::new("wsl.exe");
    command.args(wsl::wsl_execution_arguments(distro, linux_path, script));
    let (pairs, wslenv) = wsl::wsl_env_assignment(&env);
    for (name, value) in &pairs {
        command.env(name, value);
    }
    if let Some(wslenv) = wslenv {
        command.env("WSLENV", wslenv);
    }
    hide_console_window(&mut command);
    Ok(command)
}

pub(crate) fn build_execution_command(
    target: &ExecutionTarget,
    script: &str,
    working_dir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Command, String> {
    #[cfg(target_os = "windows")]
    {
        build_windows_command(target, script, working_dir, env)
    }

    #[cfg(not(target_os = "windows"))]
    {
        match target {
            ExecutionTarget::Native { .. } => {
                let mut command = shell_command(script);
                command.current_dir(working_dir).envs(env);
                Ok(command)
            }
            ExecutionTarget::Wsl { .. } => Err("WSL ist nur unter Windows verfügbar.".to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandShellInfo {
    pub(crate) id: String,
    pub(crate) available: bool,
    pub(crate) path: Option<String>,
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_command_shells() -> Vec<CommandShellInfo> {
    let entries = [
        (NativeShell::Cmd, "cmd"),
        (NativeShell::PowerShell7, "powerShell7"),
        (NativeShell::WindowsPowerShell, "windowsPowerShell"),
    ];

    entries
        .into_iter()
        .map(|(shell, id)| {
            let resolved = resolve_windows_shell(shell);
            CommandShellInfo {
                id: id.to_string(),
                available: resolved.is_some(),
                path: resolved.as_deref().map(display_path),
            }
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_command_shells() -> Vec<CommandShellInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_shape_defaults_missing_shell_to_platform_default() {
        let target: ExecutionTarget = serde_json::from_str(r#"{"type":"native"}"#).unwrap();
        assert_eq!(
            target,
            ExecutionTarget::Native {
                shell: NativeShell::PlatformDefault
            }
        );
    }

    #[test]
    fn config_shape_round_trips_explicit_shell() {
        let json = r#"{"type":"native","shell":"powerShell7"}"#;
        let target: ExecutionTarget = serde_json::from_str(json).unwrap();
        assert_eq!(
            target,
            ExecutionTarget::Native {
                shell: NativeShell::PowerShell7
            }
        );
        assert_eq!(serde_json::to_string(&target).unwrap(), json);
    }

    #[test]
    fn config_shape_round_trips_wsl_target() {
        let json = r#"{"type":"wsl","distro":"Ubuntu","linuxPath":"/home/julien/dev/foo"}"#;
        let target: ExecutionTarget = serde_json::from_str(json).unwrap();
        assert_eq!(
            target,
            ExecutionTarget::Wsl {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/julien/dev/foo".to_string(),
            }
        );
        assert_eq!(serde_json::to_string(&target).unwrap(), json);
    }

    #[cfg(target_os = "windows")]
    mod windows {
        use super::super::*;
        use std::path::Path;

        #[test]
        fn platform_default_matches_existing_cmd_behavior() {
            let target = ExecutionTarget::default();
            let command = build_execution_command(
                &target,
                "pnpm dev",
                Path::new("C:\\dev\\foo"),
                &BTreeMap::new(),
            )
            .unwrap();

            assert_eq!(command.get_program(), "cmd.exe");
            let args: Vec<_> = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                vec![
                    "/D".to_string(),
                    "/S".to_string(),
                    "/C".to_string(),
                    "chcp 65001>nul 2>nul & pnpm dev".to_string(),
                ]
            );
        }

        #[test]
        fn windows_shell_executable_returns_the_three_literal_names() {
            assert_eq!(windows_shell_executable(NativeShell::Cmd), "cmd.exe");
            assert_eq!(
                windows_shell_executable(NativeShell::PowerShell7),
                "pwsh.exe"
            );
            assert_eq!(
                windows_shell_executable(NativeShell::WindowsPowerShell),
                "powershell.exe"
            );
        }

        #[test]
        fn powershell_arguments_build_the_exact_argv() {
            assert_eq!(
                powershell_arguments("pnpm dev"),
                vec![
                    "-NoLogo".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$LASTEXITCODE = 0\npnpm dev\nif (-not $?) { exit $(if ($LASTEXITCODE) { $LASTEXITCODE } else { 1 }) }\nexit $LASTEXITCODE".to_string(),
                ]
            );
        }

        #[test]
        fn powershell_arguments_never_disable_the_profile() {
            let args = powershell_arguments("pnpm dev");
            assert!(!args.contains(&"-NoProfile".to_string()));
        }

        #[test]
        fn powershell_arguments_keep_a_quoted_script_as_one_element() {
            let script = r#"pnpm run "say \"hi\"""#;
            let args = powershell_arguments(script);
            assert_eq!(args.last().unwrap(), &powershell_script(script));
        }

        #[test]
        fn powershell_arguments_propagate_the_native_exit_code() {
            let args = powershell_arguments("cmd /c exit 3");
            let script = args.last().unwrap();
            assert!(script.contains("exit $LASTEXITCODE"));
            assert!(script.contains("if (-not $?)"));
        }

        #[test]
        fn shell_not_found_message_uses_the_command_headline() {
            let message = shell_not_found_message(
                "Command konnte nicht gestartet werden.",
                NativeShell::PowerShell7,
                "pwsh.exe",
                Path::new("C:\\dev\\foo"),
            );
            assert_eq!(
                message,
                "Command konnte nicht gestartet werden.\n\n\
Umgebung: Windows\n\
Shell: PowerShell 7\n\
Programm: pwsh.exe\n\
Projekt: C:\\dev\\foo\n\n\
pwsh.exe wurde nicht gefunden. Wähle in den Einstellungen eine andere Command-Shell."
            );
        }

        #[test]
        fn shell_not_found_message_uses_the_terminal_headline() {
            let message = shell_not_found_message(
                "Terminal konnte nicht geöffnet werden.",
                NativeShell::PowerShell7,
                "pwsh.exe",
                Path::new("C:\\dev\\foo"),
            );
            assert_eq!(
                message,
                "Terminal konnte nicht geöffnet werden.\n\n\
Umgebung: Windows\n\
Shell: PowerShell 7\n\
Programm: pwsh.exe\n\
Projekt: C:\\dev\\foo\n\n\
pwsh.exe wurde nicht gefunden. Wähle in den Einstellungen eine andere Command-Shell."
            );
        }

        #[test]
        fn build_execution_command_for_wsl_uses_wsl_exe() {
            let target = ExecutionTarget::Wsl {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/julien/dev/foo".to_string(),
            };
            let command = build_execution_command(
                &target,
                "pnpm dev",
                Path::new("C:\\dev\\foo"),
                &BTreeMap::new(),
            )
            .unwrap();

            assert_eq!(command.get_program(), "wsl.exe");
            let args: Vec<_> = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                vec![
                    "-d".to_string(),
                    "Ubuntu".to_string(),
                    "--cd".to_string(),
                    "/home/julien/dev/foo".to_string(),
                    "--exec".to_string(),
                    "bash".to_string(),
                    "-lc".to_string(),
                    "pnpm dev".to_string(),
                ]
            );
        }

        #[test]
        fn build_execution_command_for_wsl_sets_env_via_command_env_and_wslenv() {
            let target = ExecutionTarget::Wsl {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/julien".to_string(),
            };
            let mut env = BTreeMap::new();
            env.insert("PORT".to_string(), "5173".to_string());
            let command =
                build_execution_command(&target, "pnpm dev", Path::new("C:\\dev\\foo"), &env)
                    .unwrap();

            let args: Vec<_> = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            assert!(
                !args.iter().any(|value| value.contains("PORT")),
                "a configured variable must never sit in the command line: {args:?}"
            );
            assert!(command.get_envs().any(|(name, value)| {
                name == std::ffi::OsStr::new("PORT") && value == Some(std::ffi::OsStr::new("5173"))
            }));
            assert!(command.get_envs().any(|(name, value)| {
                name == std::ffi::OsStr::new("WSLENV")
                    && value == Some(std::ffi::OsStr::new("PORT"))
            }));
        }

        #[test]
        fn build_execution_command_for_wsl_rejects_an_invalid_env_name() {
            let target = ExecutionTarget::Wsl {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/julien".to_string(),
            };
            let mut env = BTreeMap::new();
            env.insert("A B".to_string(), "x".to_string());
            let error =
                build_execution_command(&target, "pnpm dev", Path::new("C:\\dev\\foo"), &env)
                    .unwrap_err();
            assert_eq!(error, "Ungültiger Name für eine Umgebungsvariable: A B");
        }

        #[test]
        fn build_execution_command_for_windows_native_applies_env() {
            let target = ExecutionTarget::default();
            let mut env = BTreeMap::new();
            env.insert("PORT".to_string(), "5173".to_string());
            let command =
                build_execution_command(&target, "pnpm dev", Path::new("C:\\dev\\foo"), &env)
                    .unwrap();
            assert!(command.get_envs().any(|(name, value)| {
                name == std::ffi::OsStr::new("PORT") && value == Some(std::ffi::OsStr::new("5173"))
            }));
        }

        #[test]
        fn build_execution_command_for_wsl_rejects_an_empty_distro() {
            let target = ExecutionTarget::Wsl {
                distro: String::new(),
                linux_path: "/home/julien/dev/foo".to_string(),
            };
            let error = build_execution_command(
                &target,
                "pnpm dev",
                Path::new("C:\\dev\\foo"),
                &BTreeMap::new(),
            )
            .unwrap_err();

            assert_eq!(
                error,
                "Command konnte nicht gestartet werden.\n\n\
Umgebung: WSL\n\
Distribution: —\n\
Pfad: /home/julien/dev/foo\n\n\
Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus."
            );
        }
    }

    #[cfg(all(test, not(target_os = "windows")))]
    mod non_windows {
        use super::*;
        use std::path::Path;

        #[test]
        fn build_execution_command_uses_the_login_shell() {
            let target = ExecutionTarget::default();
            let command = build_execution_command(
                &target,
                "pnpm dev",
                Path::new("/tmp/foo"),
                &BTreeMap::new(),
            )
            .unwrap();

            assert_eq!(command.get_program(), "/bin/sh");
            let args: Vec<_> = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            assert_eq!(args, vec!["-lc".to_string(), "pnpm dev".to_string()]);
        }

        #[test]
        fn build_execution_command_rejects_wsl_off_windows() {
            let target = ExecutionTarget::Wsl {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/julien/dev/foo".to_string(),
            };
            let error = build_execution_command(
                &target,
                "pnpm dev",
                Path::new("/tmp/foo"),
                &BTreeMap::new(),
            )
            .unwrap_err();

            assert_eq!(error, "WSL ist nur unter Windows verfügbar.");
        }
    }
}
