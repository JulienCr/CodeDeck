use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "windows")]
use crate::platform::launchers::hide_console_window;
use crate::platform::launchers::shell_command;
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
}

impl Default for ExecutionTarget {
    fn default() -> Self {
        ExecutionTarget::Native {
            shell: NativeShell::PlatformDefault,
        }
    }
}

#[cfg(target_os = "windows")]
const POWERSHELL_UTF8_PREFIX: &str = "chcp 65001 > $null; ";

#[cfg(target_os = "windows")]
fn windows_shell_executable(shell: NativeShell) -> &'static str {
    match shell {
        NativeShell::PlatformDefault | NativeShell::Cmd => "cmd.exe",
        NativeShell::PowerShell7 => "pwsh.exe",
        NativeShell::WindowsPowerShell => "powershell.exe",
    }
}

#[cfg(target_os = "windows")]
fn powershell_arguments(script: &str) -> Vec<String> {
    vec![
        "-NoLogo".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        format!("{POWERSHELL_UTF8_PREFIX}{script}"),
    ]
}

#[cfg(target_os = "windows")]
fn resolve_windows_shell(shell: NativeShell) -> Option<PathBuf> {
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
fn shell_not_found_message(shell: NativeShell, executable: &str, working_dir: &Path) -> String {
    format!(
        "Command konnte nicht gestartet werden.\n\nUmgebung: Windows\nShell: {}\nProgramm: {executable}\nProjekt: {}\n\n{executable} wurde nicht gefunden. Wähle in den Einstellungen eine andere Command-Shell.",
        shell.german_display_name(),
        display_path(working_dir)
    )
}

#[cfg(target_os = "windows")]
fn build_windows_command(
    target: &ExecutionTarget,
    script: &str,
    working_dir: &Path,
) -> Result<Command, String> {
    let ExecutionTarget::Native { shell } = target;

    if matches!(shell, NativeShell::PlatformDefault | NativeShell::Cmd) {
        let mut command = shell_command(script);
        command.current_dir(working_dir);
        return Ok(command);
    }

    let executable = windows_shell_executable(*shell);
    let program = resolve_windows_shell(*shell)
        .ok_or_else(|| shell_not_found_message(*shell, executable, working_dir))?;

    let mut command = Command::new(&program);
    command
        .args(powershell_arguments(script))
        .current_dir(working_dir)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    hide_console_window(&mut command);
    Ok(command)
}

pub(crate) fn build_execution_command(
    target: &ExecutionTarget,
    script: &str,
    working_dir: &Path,
) -> Result<Command, String> {
    #[cfg(target_os = "windows")]
    {
        build_windows_command(target, script, working_dir)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = target;
        let mut command = shell_command(script);
        command.current_dir(working_dir);
        Ok(command)
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

    #[cfg(target_os = "windows")]
    mod windows {
        use super::super::*;
        use std::path::Path;

        #[test]
        fn platform_default_matches_existing_cmd_behavior() {
            let target = ExecutionTarget::default();
            let command =
                build_execution_command(&target, "pnpm dev", Path::new("C:\\dev\\foo")).unwrap();

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
                    "chcp 65001 > $null; pnpm dev".to_string(),
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
            assert_eq!(
                args.last().unwrap(),
                &format!("{POWERSHELL_UTF8_PREFIX}{script}")
            );
        }

        #[test]
        fn shell_not_found_message_matches_the_diagnostic_format() {
            let message = shell_not_found_message(
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
    }
}
