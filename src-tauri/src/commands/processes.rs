use std::collections::HashMap;

use tauri::{AppHandle, State};

use crate::{
    platform::execution::ExecutionTarget,
    process::{
        manager,
        state::{ProcessRegistry, ProcessStarted},
    },
};

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_process(
    app: AppHandle,
    run_id: String,
    project_path: String,
    command: String,
    working_dir: Option<String>,
    env: HashMap<String, String>,
    label: String,
    notify_on_exit: bool,
    execution_target: Option<ExecutionTarget>,
) -> Result<ProcessStarted, String> {
    manager::start_process(
        app,
        run_id,
        project_path,
        command,
        working_dir,
        env,
        label,
        notify_on_exit,
        execution_target,
    )
}

#[tauri::command]
pub(crate) fn stop_process(
    run_id: String,
    pid: u32,
    registry: State<'_, ProcessRegistry>,
) -> Result<(), String> {
    manager::stop_process(&run_id, pid, registry.inner())
}
