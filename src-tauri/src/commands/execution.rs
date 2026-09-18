use crate::platform::execution::{self, CommandShellInfo};
#[cfg(target_os = "windows")]
use crate::platform::wsl;
use serde::Serialize;

#[tauri::command]
pub(crate) fn detect_command_shells() -> Vec<CommandShellInfo> {
    execution::detect_command_shells()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WslLocation {
    pub(crate) distro: String,
    pub(crate) linux_path: String,
    pub(crate) host_path: String,
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub(crate) fn detect_wsl_distros() -> Vec<String> {
    wsl::detect_wsl_distros()
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(crate) fn detect_wsl_distros() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub(crate) fn resolve_wsl_location(
    path: String,
    distro: Option<String>,
) -> Result<WslLocation, String> {
    // A UNC input is rebuilt rather than echoed, so the \\wsl$\ spelling and
    // any stray separators come back in the one form Explorer and the IDEs
    // are given.
    if let Some((unc_distro, linux_path)) = wsl::parse_wsl_unc_path(&path) {
        let host_path = wsl::wsl_unc_path(&unc_distro, &linux_path);
        return Ok(WslLocation {
            distro: unc_distro,
            linux_path,
            host_path,
        });
    }

    let distro = distro
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            wsl::wsl_error_message(
                "Pfad konnte nicht aufgelöst werden.",
                "",
                "",
                "Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus.",
            )
        })?;

    let linux_path = wsl::resolve_linux_path(&distro, &path)?;
    Ok(WslLocation {
        distro,
        linux_path,
        host_path: path,
    })
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(crate) fn resolve_wsl_location(
    path: String,
    distro: Option<String>,
) -> Result<WslLocation, String> {
    let _ = (path, distro);
    Err("WSL ist nur unter Windows verfügbar.".to_string())
}
