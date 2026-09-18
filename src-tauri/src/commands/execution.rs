use crate::platform::execution::{self, CommandShellInfo};

#[tauri::command]
pub(crate) fn detect_command_shells() -> Vec<CommandShellInfo> {
    execution::detect_command_shells()
}
