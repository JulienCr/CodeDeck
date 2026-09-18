use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "windows")]
use crate::platform::launchers::hide_console_window;
#[cfg(target_os = "windows")]
use crate::platform::wsl;
use crate::{
    platform::{
        execution::{build_execution_command, ExecutionTarget},
        notifications::send_system_notification,
    },
    process::{
        ansi::strip_ansi_codes,
        decode::decode_output_bytes,
        state::{
            ProcessExitEvent, ProcessHandle, ProcessOutputEvent, ProcessRegistry, ProcessStarted,
        },
    },
    projects::validation::display_path,
};

/// Injected by the (platform-gated) caller so this reader stays free of any
/// `#[cfg]`: on WSL it checks a line against the pgid sentinel and records the
/// match in the registry; native runs simply never construct one.
struct PgidCapture {
    try_capture: Box<dyn Fn(&str) -> bool + Send>,
}

fn stream_process_output<R>(
    reader: R,
    app: AppHandle,
    run_id: String,
    stream: &'static str,
    capture: Option<PgidCapture>,
) where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();

        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => {
                    while matches!(bytes.last().copied(), Some(b'\n') | Some(b'\r')) {
                        bytes.pop();
                    }
                    let line = strip_ansi_codes(decode_output_bytes(&bytes));

                    if let Some(capture) = &capture {
                        if (capture.try_capture)(&line) {
                            continue;
                        }
                    }

                    let _ = app.emit(
                        "code-deck://process-output",
                        ProcessOutputEvent {
                            run_id: run_id.clone(),
                            stream: stream.to_string(),
                            line,
                        },
                    );
                }
                Err(error) => {
                    let _ = app.emit(
                        "code-deck://process-output",
                        ProcessOutputEvent {
                            run_id: run_id.clone(),
                            stream: "stderr".to_string(),
                            line: format!(
                                "[Code Deck] Output konnte nicht gelesen werden: {error}"
                            ),
                        },
                    );
                    break;
                }
            }
        }
    });
}

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
    if command.trim().is_empty() {
        return Err("Der Command ist leer.".to_string());
    }

    let project_root = PathBuf::from(&project_path);
    if !project_root.is_dir() {
        return Err(format!("Projektordner nicht gefunden: {project_path}"));
    }

    let working_dir_trimmed = working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let run_dir = working_dir_trimmed
        .map(PathBuf::from)
        .map(|value| {
            if value.is_absolute() {
                value
            } else {
                project_root.join(&value)
            }
        })
        .unwrap_or_else(|| project_root.clone());

    let target = execution_target.unwrap_or_default();

    // A WSL working directory is resolved on the Linux side; when it is
    // already Linux-absolute there is no host path to check `is_dir()` on.
    #[cfg(target_os = "windows")]
    let (target, skip_run_dir_check) = match target {
        ExecutionTarget::Wsl { distro, linux_path } => {
            let resolved_linux_path = wsl::wsl_working_directory(&linux_path, working_dir_trimmed)?;
            let linux_absolute = working_dir_trimmed.is_some_and(|value| value.starts_with('/'));
            (
                ExecutionTarget::Wsl {
                    distro,
                    linux_path: resolved_linux_path,
                },
                linux_absolute,
            )
        }
        native @ ExecutionTarget::Native { .. } => (native, false),
    };
    #[cfg(not(target_os = "windows"))]
    let skip_run_dir_check = false;

    if !skip_run_dir_check && !run_dir.is_dir() {
        return Err(format!(
            "Arbeitsverzeichnis nicht gefunden: {}",
            display_path(&run_dir)
        ));
    }

    let env: BTreeMap<String, String> = env.into_iter().collect();

    // The sentinel line is prepended to the WSL script only; native runs pass
    // the command through unchanged and never build a capture.
    #[cfg(target_os = "windows")]
    let (script, capture) = match &target {
        ExecutionTarget::Wsl { .. } => {
            let marker = wsl::pgid_marker(&run_id);
            let scripted = wsl::script_with_pgid_marker(&marker, &command);
            let app_for_capture = app.clone();
            let run_id_for_capture = run_id.clone();
            let capture = PgidCapture {
                try_capture: Box::new(move |line: &str| {
                    match wsl::parse_pgid_line(&marker, line) {
                        Some(pgid) => {
                            app_for_capture
                                .state::<ProcessRegistry>()
                                .set_pgid(&run_id_for_capture, pgid);
                            true
                        }
                        None => false,
                    }
                }),
            };
            (scripted, Some(capture))
        }
        ExecutionTarget::Native { .. } => (command.clone(), None),
    };
    #[cfg(not(target_os = "windows"))]
    let (script, capture): (String, Option<PgidCapture>) = (command.clone(), None);

    let mut process = build_execution_command(&target, &script, &run_dir, &env)?;
    process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = process
        .spawn()
        .map_err(|error| format!("Command konnte nicht gestartet werden: {error}"))?;
    let pid = child.id();

    let handle = match &target {
        ExecutionTarget::Native { .. } => ProcessHandle::Native { pid },
        ExecutionTarget::Wsl { distro, .. } => ProcessHandle::Wsl {
            launcher_pid: pid,
            distro: distro.clone(),
            pgid: None,
        },
    };
    app.state::<ProcessRegistry>()
        .register(run_id.clone(), handle);

    if let Some(stdout) = child.stdout.take() {
        stream_process_output(stdout, app.clone(), run_id.clone(), "stdout", capture);
    }

    if let Some(stderr) = child.stderr.take() {
        stream_process_output(stderr, app.clone(), run_id.clone(), "stderr", None);
    }

    thread::spawn(move || {
        let status = child.wait();
        let (exit_code, success) = match status {
            Ok(status) => (status.code(), status.success()),
            Err(_) => (None, false),
        };
        app.state::<ProcessRegistry>().take(&run_id);
        let _ = app.emit(
            "code-deck://process-exit",
            ProcessExitEvent {
                run_id,
                exit_code,
                success,
            },
        );
        if notify_on_exit {
            let body = if success {
                format!("{label} wurde erfolgreich beendet.")
            } else {
                match exit_code {
                    Some(code) => format!("{label} ist mit Exit-Code {code} fehlgeschlagen."),
                    None => format!("{label} wurde unerwartet beendet."),
                }
            };
            send_system_notification("Code Deck", &body);
        }
    });

    Ok(ProcessStarted { pid })
}

pub(crate) fn stop_process(
    run_id: &str,
    pid: u32,
    registry: &ProcessRegistry,
) -> Result<(), String> {
    match registry.get(run_id) {
        #[cfg(target_os = "windows")]
        Some(ProcessHandle::Wsl {
            launcher_pid,
            distro,
            pgid: Some(pgid),
        }) => {
            stop_wsl_process_group(&distro, pgid)?;
            // The launcher normally exits by itself once the group dies, so
            // taskkill reporting "process not found" here is the happy path,
            // not a failure — its result must not surface as an error.
            let _ = taskkill(launcher_pid);
            Ok(())
        }
        #[cfg(target_os = "windows")]
        Some(ProcessHandle::Wsl {
            launcher_pid,
            pgid: None,
            ..
        }) => taskkill(launcher_pid),
        _ => stop_native_process(pid),
    }
}

#[cfg(target_os = "windows")]
fn stop_wsl_process_group(distro: &str, pgid: i32) -> Result<(), String> {
    let script = wsl::stop_group_script(pgid);
    let mut command = Command::new("wsl.exe");
    command
        .args(["-d", distro, "--exec", "bash", "-lc", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("Prozess konnte nicht beendet werden: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        // The script itself always exits 0, so a failure here comes from
        // wsl.exe, whose own messages are UTF-16LE rather than child output.
        let detail = wsl::decode_wsl_output(&output.stderr);
        Err(wsl::wsl_error_message(
            "Prozess konnte nicht beendet werden.",
            distro,
            "",
            detail.trim(),
        ))
    }
}

#[cfg(target_os = "windows")]
fn taskkill(pid: u32) -> Result<(), String> {
    let mut command = Command::new("taskkill");
    command
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console_window(&mut command);
    let status = command
        .status()
        .map_err(|error| format!("Prozess konnte nicht beendet werden: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("taskkill meldete einen Fehler für PID {pid}."))
    }
}

#[cfg(target_os = "windows")]
fn stop_native_process(pid: u32) -> Result<(), String> {
    taskkill(pid)
}

#[cfg(not(target_os = "windows"))]
fn stop_native_process(pid: u32) -> Result<(), String> {
    let group = format!("-{pid}");
    let status = Command::new("kill")
        .args(["-TERM", &group])
        .status()
        .map_err(|error| format!("Prozess konnte nicht beendet werden: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        let fallback = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|error| format!("Prozess konnte nicht beendet werden: {error}"))?;
        fallback
            .success()
            .then_some(())
            .ok_or_else(|| format!("kill meldete einen Fehler für PID {pid}."))
    }
}
