use crate::platform::launchers::hide_console_window;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

const WSL_SHELL: &str = "bash";

/// `wsl.exe` writes UTF-16LE with no guaranteed BOM. Decoding it as UTF-8
/// instead silently yields a NUL-filled string that still passes
/// `is_empty()`, so the byte shape (even length, NUL at an odd index) is
/// checked before trusting either interpretation.
pub(crate) fn decode_wsl_output(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    let is_utf16le = !bytes.is_empty()
        && bytes.len().is_multiple_of(2)
        && bytes.iter().skip(1).step_by(2).any(|&byte| byte == 0);

    if !is_utf16le {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

pub(crate) fn parse_distro_list(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.split('\n')
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn collapse_slashes(value: &str) -> String {
    let mut result = String::new();
    let mut last_was_slash = false;
    for character in value.chars() {
        if character == '/' {
            if !last_was_slash {
                result.push('/');
            }
            last_was_slash = true;
        } else {
            result.push(character);
            last_was_slash = false;
        }
    }
    result.trim_end_matches('/').to_string()
}

pub(crate) fn parse_wsl_unc_path(path: &str) -> Option<(String, String)> {
    let normalized = path.replace('\\', "/");
    let rest = normalized.strip_prefix("//")?;
    let mut segments = rest.splitn(3, '/');
    let host = segments.next()?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && !host.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let distro = segments.next().filter(|value| !value.is_empty())?;
    let tail = segments.next().unwrap_or("");
    Some((distro.to_string(), format!("/{}", collapse_slashes(tail))))
}

pub(crate) fn wsl_unc_path(distro: &str, linux_path: &str) -> String {
    let trimmed = linux_path.trim_start_matches('/');
    if trimmed.is_empty() {
        format!(r"\\wsl.localhost\{distro}")
    } else {
        format!(r"\\wsl.localhost\{distro}\{}", trimmed.replace('/', "\\"))
    }
}

/// A Windows path component can never contain `/`, so replacing `\` is
/// lossless. Belt-and-braces: backslashes were measured not to survive
/// argument passing into `wslpath` on this machine.
pub(crate) fn wslpath_argument(windows_path: &str) -> String {
    windows_path.replace('\\', "/")
}

pub(crate) fn join_linux_path(base: &str, relative: &str) -> String {
    let relative = relative.trim().replace('\\', "/");
    if relative.is_empty() || relative == "." {
        return collapse_slashes(base);
    }
    collapse_slashes(&format!("{base}/{relative}"))
}

pub(crate) fn is_valid_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Native accepts whatever env name the host shell tolerates, so an existing
/// project keeps working; only the WSL path (where names become independent
/// `env` argv elements) rejects one that would not parse as `NAME=VALUE`.
pub(crate) fn validated_env(
    env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    for name in env.keys() {
        if !is_valid_env_name(name) {
            return Err(format!(
                "Ungültiger Name für eine Umgebungsvariable: {name}"
            ));
        }
    }
    Ok(env.clone())
}

fn is_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let has_drive_letter = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    has_drive_letter || value.starts_with(r"\\")
}

/// The stored `linux_path` is the project root; a relative working directory
/// is resolved against it, a Linux-absolute one is used as-is, and a Windows
/// path is rejected rather than silently treated as a Linux path fragment.
pub(crate) fn wsl_working_directory(
    linux_path: &str,
    working_dir: Option<&str>,
) -> Result<String, String> {
    let working_dir = working_dir.map(str::trim).unwrap_or("");
    if working_dir.is_empty() || working_dir == "." {
        return Ok(linux_path.to_string());
    }
    if working_dir.starts_with('/') {
        return Ok(working_dir.to_string());
    }
    if is_windows_absolute_path(working_dir) {
        return Err(format!(
            "Arbeitsverzeichnis konnte nicht aufgelöst werden.\n\nEin absoluter Windows-Pfad kann nicht als Arbeitsverzeichnis eines WSL-Commands verwendet werden: {working_dir}\n\nVerwende einen relativen Pfad oder einen Linux-Pfad."
        ));
    }
    Ok(join_linux_path(linux_path, working_dir))
}

/// Argv after the program name. `--exec` re-parses nothing through the
/// user's login shell, unlike a bare `--`; env pairs are independent argv
/// elements, never interpolated into `script`, so no value can break out.
pub(crate) fn wsl_execution_arguments(
    distro: &str,
    linux_path: &str,
    env: &BTreeMap<String, String>,
    script: &str,
) -> Vec<String> {
    let mut arguments = vec![
        "-d".to_string(),
        distro.to_string(),
        "--cd".to_string(),
        linux_path.to_string(),
        "--exec".to_string(),
    ];

    if !env.is_empty() {
        arguments.push("env".to_string());
        for (name, value) in env {
            arguments.push(format!("{name}={value}"));
        }
    }

    arguments.push(WSL_SHELL.to_string());
    arguments.push("-lc".to_string());
    arguments.push(script.to_string());
    arguments
}

/// Argv for a git invocation. `--exec` runs `env` then `git` directly, never
/// a login shell, so a caller-supplied `--` (e.g. `git add -- file`) reaches
/// git verbatim instead of being mistaken for the `wsl.exe` separator.
pub(crate) fn wsl_git_arguments(
    distro: &str,
    linux_path: &str,
    env: &[(&str, &str)],
    args: &[&str],
) -> Vec<String> {
    let mut arguments = vec![
        "-d".to_string(),
        distro.to_string(),
        "--cd".to_string(),
        linux_path.to_string(),
        "--exec".to_string(),
        "env".to_string(),
    ];
    arguments.extend(env.iter().map(|(name, value)| format!("{name}={value}")));
    arguments.push("git".to_string());
    arguments.extend(args.iter().map(|value| value.to_string()));
    arguments
}

pub(crate) fn wsl_test_path_arguments(distro: &str, path: &str) -> Vec<String> {
    vec![
        "-d".to_string(),
        distro.to_string(),
        "--exec".to_string(),
        "test".to_string(),
        "-e".to_string(),
        path.to_string(),
    ]
}

pub(crate) fn stop_group_script(pgid: i32) -> String {
    format!(
        "kill -TERM -- -{pgid} 2>/dev/null; \
for _ in $(seq 15); do kill -0 -- -{pgid} 2>/dev/null || exit 0; sleep 0.1; done; \
kill -KILL -- -{pgid} 2>/dev/null; exit 0"
    )
}

/// The launcher's Windows PID says nothing about the Linux side, so the
/// script prints this on its own first line and the stdout reader consumes
/// it instead of emitting it. The run id is sanitised so it is safe to embed
/// directly in shell source.
pub(crate) fn pgid_marker(run_id: &str) -> String {
    let sanitized: String = run_id
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '_' || *character == '-'
        })
        .collect();
    format!("code-deck-pgid:{sanitized}:")
}

// Newline-joined, not `;`: a user script ending in a `#` comment would
// swallow a `;`-joined marker line along with the rest of the command.
pub(crate) fn script_with_pgid_marker(marker: &str, script: &str) -> String {
    format!("echo \"{marker}$$\"\n{script}")
}

pub(crate) fn parse_pgid_line(marker: &str, line: &str) -> Option<i32> {
    let tail = line.strip_prefix(marker)?;
    if tail.is_empty() || !tail.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    tail.parse::<i32>().ok().filter(|value| *value > 0)
}

pub(crate) fn is_distro_not_found(exit_code: Option<i32>, stderr: &str) -> bool {
    exit_code == Some(-1) || stderr.contains("WSL_E_DISTRO_NOT_FOUND")
}

pub(crate) fn wsl_error_message(
    headline: &str,
    distro: &str,
    linux_path: &str,
    detail: &str,
) -> String {
    let distro_display = if distro.is_empty() { "—" } else { distro };
    let path_display = if linux_path.is_empty() {
        "—"
    } else {
        linux_path
    };
    format!(
        "{headline}\n\nUmgebung: WSL\nDistribution: {distro_display}\nPfad: {path_display}\n\n{detail}"
    )
}

fn wsl_command() -> Command {
    let mut command = Command::new("wsl.exe");
    command.stdin(Stdio::null());
    hide_console_window(&mut command);
    command
}

pub(crate) fn detect_wsl_distros() -> Vec<String> {
    let Ok(output) = wsl_command().args(["--list", "--quiet"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_distro_list(&decode_wsl_output(&output.stdout))
}

/// Handing an already-Linux path to `wslpath -a` returns a plausible, wrong
/// `/mnt/<drive>/...` answer with exit code 0 — measured, never an error.
/// Refusing before the call is the only way to catch it.
pub(crate) fn resolve_linux_path(distro: &str, windows_path: &str) -> Result<String, String> {
    if windows_path.starts_with('/') || parse_wsl_unc_path(windows_path).is_some() {
        return Err(wsl_error_message(
            "Pfad konnte nicht aufgelöst werden.",
            distro,
            "",
            "Der Pfad ist bereits ein Linux-Pfad und muss nicht konvertiert werden.",
        ));
    }

    let argument = wslpath_argument(windows_path);
    let output = wsl_command()
        .args(["-d", distro, "--exec", "wslpath", "-a", &argument])
        .output()
        .map_err(|_| {
            wsl_error_message(
                "Pfad konnte nicht aufgelöst werden.",
                distro,
                "",
                "wsl.exe wurde nicht gefunden. WSL ist auf diesem System nicht verfügbar.",
            )
        })?;

    let stderr = decode_wsl_output(&output.stderr);
    if !output.status.success() || is_distro_not_found(output.status.code(), &stderr) {
        return Err(wsl_error_message(
            "Pfad konnte nicht aufgelöst werden.",
            distro,
            "",
            "Die ausgewählte WSL-Distribution ist nicht verfügbar.",
        ));
    }

    Ok(decode_wsl_output(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_wsl_output_reads_the_measured_bytes() {
        let bytes = [
            0x55, 0x00, 0x62, 0x00, 0x75, 0x00, 0x6E, 0x00, 0x74, 0x00, 0x75, 0x00, 0x0D, 0x00,
            0x0A, 0x00,
        ];
        assert_eq!(decode_wsl_output(&bytes), "Ubuntu\r\n");
    }

    #[test]
    fn decode_wsl_output_strips_a_leading_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend([0x55, 0x00, 0x62, 0x00]);
        assert_eq!(decode_wsl_output(&bytes), "Ub");
    }

    #[test]
    fn decode_wsl_output_falls_back_to_utf8() {
        assert_eq!(decode_wsl_output(b"hello!"), "hello!");
    }

    #[test]
    fn decode_wsl_output_handles_empty_input() {
        assert_eq!(decode_wsl_output(&[]), "");
    }

    #[test]
    fn parse_distro_list_splits_and_trims() {
        assert_eq!(
            parse_distro_list("Ubuntu\r\ndocker-desktop\r\n"),
            vec!["Ubuntu".to_string(), "docker-desktop".to_string()]
        );
    }

    #[test]
    fn parse_distro_list_strips_leading_bom() {
        assert_eq!(
            parse_distro_list("\u{feff}Ubuntu\r\n"),
            vec!["Ubuntu".to_string()]
        );
    }

    #[test]
    fn parse_distro_list_drops_blank_lines() {
        assert_eq!(
            parse_distro_list("Ubuntu\r\n\r\ndocker-desktop\r\n\r\n"),
            vec!["Ubuntu".to_string(), "docker-desktop".to_string()]
        );
    }

    #[test]
    fn parse_wsl_unc_path_reads_wsl_localhost() {
        assert_eq!(
            parse_wsl_unc_path(r"\\wsl.localhost\Ubuntu\home\julien\dev\foo"),
            Some(("Ubuntu".to_string(), "/home/julien/dev/foo".to_string()))
        );
    }

    #[test]
    fn parse_wsl_unc_path_reads_wsl_dollar() {
        assert_eq!(
            parse_wsl_unc_path(r"\\wsl$\Ubuntu\home\julien"),
            Some(("Ubuntu".to_string(), "/home/julien".to_string()))
        );
    }

    #[test]
    fn parse_wsl_unc_path_accepts_forward_slashes() {
        assert_eq!(
            parse_wsl_unc_path("//wsl.localhost/Ubuntu/home/julien"),
            Some(("Ubuntu".to_string(), "/home/julien".to_string()))
        );
    }

    #[test]
    fn parse_wsl_unc_path_distro_root_is_slash() {
        assert_eq!(
            parse_wsl_unc_path(r"\\wsl.localhost\Ubuntu"),
            Some(("Ubuntu".to_string(), "/".to_string()))
        );
    }

    #[test]
    fn parse_wsl_unc_path_rejects_a_windows_path() {
        assert_eq!(parse_wsl_unc_path(r"C:\dev\foo"), None);
    }

    #[test]
    fn parse_wsl_unc_path_rejects_a_foreign_unc_share() {
        assert_eq!(parse_wsl_unc_path(r"\\server\share\x"), None);
    }

    #[test]
    fn wsl_unc_path_round_trips_with_parse() {
        let unc = wsl_unc_path("Ubuntu", "/home/julien/dev/foo");
        assert_eq!(unc, r"\\wsl.localhost\Ubuntu\home\julien\dev\foo");
        assert_eq!(
            parse_wsl_unc_path(&unc),
            Some(("Ubuntu".to_string(), "/home/julien/dev/foo".to_string()))
        );
    }

    #[test]
    fn wsl_unc_path_round_trips_the_distro_root() {
        let unc = wsl_unc_path("Ubuntu", "/");
        assert_eq!(unc, r"\\wsl.localhost\Ubuntu");
        assert_eq!(
            parse_wsl_unc_path(&unc),
            Some(("Ubuntu".to_string(), "/".to_string()))
        );
    }

    #[test]
    fn wslpath_argument_converts_backslashes() {
        assert_eq!(wslpath_argument(r"C:\dev\my project"), "C:/dev/my project");
    }

    #[test]
    fn join_linux_path_appends_relative() {
        assert_eq!(
            join_linux_path("/home/x/foo", "packages/web"),
            "/home/x/foo/packages/web"
        );
    }

    #[test]
    fn join_linux_path_collapses_a_trailing_slash_on_the_base() {
        assert_eq!(join_linux_path("/home/x/foo/", ""), "/home/x/foo");
    }

    #[test]
    fn join_linux_path_returns_base_for_empty_relative() {
        assert_eq!(join_linux_path("/home/x/foo", "."), "/home/x/foo");
    }

    #[test]
    fn is_valid_env_name_accepts_and_rejects() {
        assert!(is_valid_env_name("_A1"));
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("1A"));
        assert!(!is_valid_env_name("A B"));
        assert!(!is_valid_env_name("A=B"));
        assert!(!is_valid_env_name("Ä"));
    }

    #[test]
    fn wsl_execution_arguments_builds_a_plain_script() {
        let args = wsl_execution_arguments(
            "Ubuntu",
            "/home/julien/dev/foo",
            &BTreeMap::new(),
            "pnpm dev",
        );
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
        assert!(args.contains(&"--exec".to_string()));
        assert!(!args.contains(&"--".to_string()));
    }

    #[test]
    fn wsl_execution_arguments_sorts_env_between_exec_and_bash() {
        let mut env = BTreeMap::new();
        env.insert("PORT".to_string(), "5173".to_string());
        env.insert("FOO".to_string(), "bar".to_string());
        let args = wsl_execution_arguments("Ubuntu", "/home/julien", &env, "pnpm dev");
        assert_eq!(
            args,
            vec![
                "-d".to_string(),
                "Ubuntu".to_string(),
                "--cd".to_string(),
                "/home/julien".to_string(),
                "--exec".to_string(),
                "env".to_string(),
                "FOO=bar".to_string(),
                "PORT=5173".to_string(),
                "bash".to_string(),
                "-lc".to_string(),
                "pnpm dev".to_string(),
            ]
        );
    }

    #[test]
    fn wsl_execution_arguments_keeps_a_quoted_script_as_one_element() {
        let script = r#"pnpm run "say \"hi\"""#;
        let args = wsl_execution_arguments("Ubuntu", "/home/julien", &BTreeMap::new(), script);
        assert_eq!(args.last().unwrap(), script);
    }

    #[test]
    fn stop_group_script_contains_term_then_kill() {
        let script = stop_group_script(1234);
        assert!(script.contains("-TERM -- -1234"));
        assert!(script.contains("-KILL -- -1234"));
        assert!(script.contains("kill -0"));
    }

    #[test]
    fn is_distro_not_found_keys_on_exit_code_or_token() {
        assert!(is_distro_not_found(Some(-1), ""));
        assert!(is_distro_not_found(
            Some(1),
            "... Wsl/Service/WSL_E_DISTRO_NOT_FOUND"
        ));
        assert!(!is_distro_not_found(Some(1), "some other failure"));
    }

    #[test]
    fn wsl_error_message_matches_the_exact_shape() {
        assert_eq!(
            wsl_error_message(
                "Command konnte nicht gestartet werden.",
                "Ubuntu",
                "/home/julien/dev/foo",
                "Die ausgewählte WSL-Distribution ist nicht verfügbar."
            ),
            "Command konnte nicht gestartet werden.\n\n\
Umgebung: WSL\n\
Distribution: Ubuntu\n\
Pfad: /home/julien/dev/foo\n\n\
Die ausgewählte WSL-Distribution ist nicht verfügbar."
        );
    }

    #[test]
    fn wsl_error_message_renders_empty_fields_as_a_dash() {
        assert_eq!(
            wsl_error_message(
                "Command konnte nicht gestartet werden.",
                "",
                "",
                "Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus."
            ),
            "Command konnte nicht gestartet werden.\n\n\
Umgebung: WSL\n\
Distribution: —\n\
Pfad: —\n\n\
Für dieses Projekt ist keine WSL-Distribution ausgewählt. Wähle in den Projekteinstellungen eine Distribution aus."
        );
    }

    #[test]
    fn validated_env_accepts_a_normal_map() {
        let mut env = BTreeMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        assert_eq!(validated_env(&env), Ok(env));
    }

    #[test]
    fn validated_env_rejects_and_names_the_offender() {
        let mut env = BTreeMap::new();
        env.insert("A B".to_string(), "x".to_string());
        assert_eq!(
            validated_env(&env),
            Err("Ungültiger Name für eine Umgebungsvariable: A B".to_string())
        );

        let mut env = BTreeMap::new();
        env.insert("1A".to_string(), "x".to_string());
        assert!(validated_env(&env).is_err());

        let mut env = BTreeMap::new();
        env.insert("A=B".to_string(), "x".to_string());
        assert!(validated_env(&env).is_err());

        let mut env = BTreeMap::new();
        env.insert(String::new(), "x".to_string());
        assert!(validated_env(&env).is_err());
    }

    #[test]
    fn wsl_working_directory_defaults_to_the_project_root() {
        assert_eq!(
            wsl_working_directory("/home/julien/dev/foo", None),
            Ok("/home/julien/dev/foo".to_string())
        );
        assert_eq!(
            wsl_working_directory("/home/julien/dev/foo", Some(".")),
            Ok("/home/julien/dev/foo".to_string())
        );
        assert_eq!(
            wsl_working_directory("/home/julien/dev/foo", Some("  ")),
            Ok("/home/julien/dev/foo".to_string())
        );
    }

    #[test]
    fn wsl_working_directory_uses_a_linux_absolute_path_verbatim() {
        assert_eq!(
            wsl_working_directory("/home/julien/dev/foo", Some("/srv/app")),
            Ok("/srv/app".to_string())
        );
    }

    #[test]
    fn wsl_working_directory_joins_a_relative_path() {
        assert_eq!(
            wsl_working_directory("/home/julien/dev/foo", Some("packages/web")),
            Ok("/home/julien/dev/foo/packages/web".to_string())
        );
    }

    #[test]
    fn wsl_working_directory_rejects_a_drive_letter_path() {
        let error = wsl_working_directory("/home/julien/dev/foo", Some(r"C:\dev\foo"))
            .expect_err("a Windows drive path must be rejected");
        assert!(error.contains(r"C:\dev\foo"));
    }

    #[test]
    fn wsl_working_directory_rejects_a_unc_path() {
        let error = wsl_working_directory("/home/julien/dev/foo", Some(r"\\server\share"))
            .expect_err("a UNC path must be rejected");
        assert!(error.contains(r"\\server\share"));
    }

    #[test]
    fn pgid_marker_strips_shell_special_characters() {
        assert_eq!(
            pgid_marker(r#"a"b$c;d e"#),
            "code-deck-pgid:abcde:".to_string()
        );
    }

    #[test]
    fn pgid_marker_keeps_a_uuid_intact() {
        let run_id = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
        assert_eq!(pgid_marker(run_id), format!("code-deck-pgid:{run_id}:"));
    }

    #[test]
    fn script_with_pgid_marker_puts_the_echo_on_its_own_first_line() {
        let script = script_with_pgid_marker("code-deck-pgid:abc:", "pnpm dev");
        let mut lines = script.lines();
        assert_eq!(lines.next(), Some(r#"echo "code-deck-pgid:abc:$$""#));
        assert_eq!(lines.next(), Some("pnpm dev"));
    }

    #[test]
    fn script_with_pgid_marker_survives_a_trailing_comment() {
        let script = script_with_pgid_marker("code-deck-pgid:abc:", "pnpm dev # comment");
        let mut lines = script.lines();
        assert_eq!(lines.next(), Some(r#"echo "code-deck-pgid:abc:$$""#));
        assert_eq!(lines.next(), Some("pnpm dev # comment"));
    }

    #[test]
    fn parse_pgid_line_accepts_marker_plus_digits() {
        assert_eq!(
            parse_pgid_line("code-deck-pgid:abc:", "code-deck-pgid:abc:1309879"),
            Some(1309879)
        );
    }

    #[test]
    fn parse_pgid_line_rejects_a_different_run_id() {
        assert_eq!(
            parse_pgid_line("code-deck-pgid:abc:", "code-deck-pgid:xyz:123"),
            None
        );
    }

    #[test]
    fn parse_pgid_line_rejects_a_non_numeric_tail() {
        assert_eq!(
            parse_pgid_line("code-deck-pgid:abc:", "code-deck-pgid:abc:notanumber"),
            None
        );
    }

    const GIT_ENV: [(&str, &str); 3] = [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_EDITOR", "true"),
        ("GIT_SEQUENCE_EDITOR", "true"),
    ];

    #[test]
    fn wsl_git_arguments_builds_the_exact_argv() {
        let args = wsl_git_arguments(
            "Ubuntu",
            "/home/julien/dev/foo",
            &GIT_ENV,
            &["status", "--short"],
        );
        assert_eq!(
            args,
            vec![
                "-d",
                "Ubuntu",
                "--cd",
                "/home/julien/dev/foo",
                "--exec",
                "env",
                "GIT_TERMINAL_PROMPT=0",
                "GIT_EDITOR=true",
                "GIT_SEQUENCE_EDITOR=true",
                "git",
                "status",
                "--short",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn wsl_git_arguments_exec_is_at_index_four() {
        let args = wsl_git_arguments("Ubuntu", "/home/julien", &GIT_ENV, &["status"]);
        assert_eq!(args[4], "--exec");
    }

    #[test]
    fn wsl_git_arguments_preserves_a_literal_double_dash_after_git() {
        let args = wsl_git_arguments(
            "Ubuntu",
            "/home/julien",
            &GIT_ENV,
            &["add", "--", "file.txt"],
        );
        let git_index = args.iter().position(|value| value == "git").unwrap();
        assert_eq!(
            &args[git_index + 1..],
            &["add".to_string(), "--".to_string(), "file.txt".to_string()]
        );
        assert_eq!(args[..git_index].iter().filter(|v| *v == "--").count(), 0);
    }

    #[test]
    fn wsl_git_arguments_keeps_each_argument_its_own_element() {
        let args = wsl_git_arguments(
            "Ubuntu",
            "/home/julien",
            &GIT_ENV,
            &["commit", "-m", "a b c"],
        );
        assert_eq!(args.last(), Some(&"a b c".to_string()));
        assert_eq!(args[args.len() - 2], "-m");
    }

    #[test]
    fn wsl_test_path_arguments_builds_the_exact_argv() {
        assert_eq!(
            wsl_test_path_arguments("Ubuntu", "/home/julien/dev/foo/.git/MERGE_HEAD"),
            vec![
                "-d",
                "Ubuntu",
                "--exec",
                "test",
                "-e",
                "/home/julien/dev/foo/.git/MERGE_HEAD",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_pgid_line_rejects_a_line_that_merely_contains_the_marker() {
        assert_eq!(
            parse_pgid_line("code-deck-pgid:abc:", "code-deck-pgid:abc:123 extra"),
            None
        );
        assert_eq!(
            parse_pgid_line("code-deck-pgid:abc:", "prefix code-deck-pgid:abc:123"),
            None
        );
    }
}
