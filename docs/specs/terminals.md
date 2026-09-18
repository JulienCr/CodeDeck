# CodeDeck — PowerShell and WSL execution support

## Goal

Add a proper execution-environment abstraction to CodeDeck so Windows projects are no longer forced to run through `cmd.exe`, and Windows users can manage both:

* native Windows projects;
* WSL projects.

A project must be able to execute its configured commands (`pnpm dev`, `pnpm build`, tests, etc.) in the same environment in which the developer normally works.

The implementation must support PowerShell profiles so tools initialized by shell configuration, such as `fnm`, are available.

Do **not** implement this as scattered special cases around `cmd.exe`. Introduce an execution abstraction that can later support additional runtimes.

## Existing behavior

On Windows, CodeDeck currently uses:

```text
cmd.exe /D /S /C <command>
```

for project commands.

The project path is used as the process working directory.

Stopping a process uses Windows `taskkill /PID <pid> /T /F`.

Git commands are executed directly with the host `git` executable.

The custom terminal setting affects the "Open Terminal" action only and must remain conceptually separate from the shell used to execute CodeDeck commands.

## Execution model

Introduce a serializable execution target.

Suggested model:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExecutionTarget {
    Native {
        shell: NativeShell,
    },
    Wsl {
        distro: String,
        linux_path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeShell {
    PlatformDefault,
    Cmd,
    PowerShell7,
    WindowsPowerShell,
}
```

On Windows:

```text
PlatformDefault -> existing cmd.exe behavior
Cmd             -> cmd.exe
PowerShell7     -> pwsh.exe
WindowsPowerShell -> powershell.exe
```

On Linux/macOS, preserve existing behavior.

Old persisted CodeDeck configurations must remain valid. Projects without an execution target must behave exactly as before.

Do not make PowerShell the new default automatically.

## Centralize process creation

Refactor the current `shell_command()` logic into something equivalent to:

```rust
build_execution_command(
    target,
    project,
    command,
    working_dir,
    env,
) -> Result<ExecutionCommand>
```

All shell-specific command construction belongs there.

Avoid duplicating WSL/PowerShell logic in frontend commands, process manager, Git management, etc.

Prefer an enum with methods over unnecessary trait/dynamic-dispatch complexity.

## Native PowerShell support

For PowerShell 7 execute through:

```text
pwsh.exe -NoLogo -Command <command>
```

For Windows PowerShell:

```text
powershell.exe -NoLogo -Command <command>
```

Important:

**Do not pass `-NoProfile`.**

The user's PowerShell profile must load normally.

This is required for environments such as:

```text
fnm
mise
volta
custom PATH modifications
shell aliases/functions
```

For example, a user may have:

```powershell
fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression
```

in their PowerShell profile.

In that case:

```text
pnpm dev
```

must work from CodeDeck exactly as it does from an ordinary PowerShell session.

Do not add:

```text
ExecutionPolicy Bypass
```

or alter the user's execution policy.

Continue capturing stdout/stderr exactly as CodeDeck currently does.

## Shell discovery

On Windows detect:

```text
cmd.exe
pwsh.exe
powershell.exe
```

The settings UI should indicate whether each executable is available.

Example:

```text
Command shell

○ Command Prompt
● PowerShell 7
○ Windows PowerShell

PowerShell 7
C:\Program Files\PowerShell\7\pwsh.exe
```

If an unavailable shell is selected, return an actionable error rather than silently falling back.

Example:

```text
PowerShell 7 (pwsh.exe) could not be found.
Choose another command shell in Settings.
```

## Settings hierarchy

Add a global setting:

```text
Settings
  Execution
    Default Windows command shell
```

Values:

```text
Platform default
Command Prompt
PowerShell 7
Windows PowerShell
```

Allow an optional per-project override:

```text
Project
  Execution environment

  Runtime: Windows
  Shell: Inherit global / CMD / PowerShell 7 / Windows PowerShell
```

The effective configuration should therefore be visible from the project details page.

A small badge on the project page/card is desirable:

```text
Windows · PowerShell 7
```

but is secondary to functionality.

---

# WSL execution backend

Only expose WSL support when running CodeDeck on Windows.

Detect WSL using `wsl.exe`.

Enumerate installed distributions using:

```text
wsl.exe --list --quiet
```

Be careful with Windows/WSL output encoding. Do not assume the result is always plain UTF-8; handle UTF-16LE/BOM/NUL-containing output if required.

Do not parse localized human-readable `wsl --status` output unless necessary.

Let the user explicitly select the distribution.

Example UI:

```text
Execution environment

○ Windows
● WSL

Distribution
[ Ubuntu            ▾ ]

Linux project path
/home/julie/dev/avolo-short
```

## WSL project paths

CodeDeck needs to distinguish:

```text
host-accessible path
```

from:

```text
execution path
```

For a Linux-filesystem WSL project:

```text
execution path:
/home/julie/dev/avolo-short
```

Windows-accessible equivalent:

```text
\\wsl.localhost\Ubuntu\home\julie\dev\avolo-short
```

Use the host/UNC path for:

```text
file inspection
README reading
package.json detection
Explorer
Windows IDEs
```

Use the Linux path for:

```text
commands
Git
terminal
build/test/dev
```

Support both:

```text
\\wsl.localhost\<distro>\...
```

and, where useful for compatibility:

```text
\\wsl$\<distro>\...
```

When importing a project from a WSL UNC path, automatically detect:

```text
distro
linux_path
```

Example:

```text
\\wsl.localhost\Ubuntu\home\julie\dev\foo
```

becomes:

```text
distro = Ubuntu
linux_path = /home/julie/dev/foo
```

## Windows-hosted projects executed through WSL

Also allow:

```text
C:\dev\foo
```

to run through WSL.

Resolve the corresponding Linux path through the selected distro, preferably with `wslpath`, e.g.:

```text
/mnt/c/dev/foo
```

Do not implement path conversion with naive string replacement.

---

# WSL command execution

Conceptually execute:

```text
wsl.exe
  -d <distro>
  --cd <linuxPath>
  --
  <shell>
  -lc
  <command>
```

Arguments such as distro and paths must be passed as process arguments, **not concatenated into a Windows shell command**.

For v1, using Bash is acceptable:

```text
bash -lc
```

but keep WSL shell selection abstract enough to support the user's default shell later.

Example:

```text
wsl.exe -d Ubuntu --cd /home/julie/dev/foo -- bash -lc "pnpm dev"
```

The actual Rust implementation should use `Command::arg()` / `args()` and must not build this as one giant escaped string.

## WSL environment variables

Existing per-command environment variables must continue to work.

For WSL, explicitly propagate configured variables to the Linux command.

Do not depend on the user's global `WSLENV`.

Environment names must be validated as shell-compatible environment variable names.

Values must not be interpolated unsafely into shell source.

---

# WSL process lifecycle

This part is important.

Do **not** consider the Windows PID of `wsl.exe` to be sufficient process ownership.

For a command such as:

```text
pnpm dev
```

CodeDeck must be able to terminate the Linux process tree cleanly.

Create the launched WSL command in its own Linux process group/session and retain its Linux PID/PGID.

Suggested runtime process representation:

```rust
pub enum ProcessHandle {
    Native {
        pid: u32,
    },
    Wsl {
        launcher_pid: u32,
        distro: String,
        pgid: i32,
    },
}
```

Stopping a WSL command should conceptually perform:

```text
kill -TERM -- -<pgid>
```

and optionally fall back to `SIGKILL` after a short timeout if necessary.

Never implement Stop as:

```text
wsl.exe --terminate Ubuntu
```

because that would kill unrelated processes belonging to the user.

After pressing Stop, `node`, `vite`, `pnpm`, etc. launched by that CodeDeck command must not remain orphaned inside WSL.

---

# Git integration

WSL projects must use the Git installation inside the selected WSL distribution.

Currently CodeDeck invokes host `git` directly. Refactor Git execution so it can use the project's `ExecutionTarget`.

For:

```text
Runtime = Windows
```

continue using:

```text
git.exe
```

For:

```text
Runtime = WSL / Ubuntu
```

execute:

```text
wsl.exe -d Ubuntu --cd <project> -- git ...
```

This applies to:

```text
status
branch
commit information
diff
stage
commit
merge/rebase detection
etc.
```

Avoid mixing Windows Git and WSL Git for the same project. The current Git implementation directly invokes the host executable, so this does require integration work.

---

# Terminal action

The existing custom terminal preference must continue to work. CodeDeck already treats terminal launching as a separate setting.

For a native Windows project with no custom terminal:

```text
CMD project            -> cmd
PowerShell 7 project   -> pwsh
Windows PowerShell     -> powershell
```

For WSL:

```text
wsl.exe -d <distro> --cd <linuxPath>
```

Using Windows Terminal automatically when available can be considered later; it is not required for the first implementation.

Extend custom terminal placeholders to support:

```text
{projectPath}
{projectName}
{wslDistro}
{wslPath}
```

For non-WSL projects the WSL placeholders should either be empty or rejected with a clear message.

---

# IDE / Explorer behavior

Do not run Windows IDEs inside WSL.

For a WSL project, the existing IDE launcher should receive the host-accessible UNC path:

```text
\\wsl.localhost\Ubuntu\home\julie\dev\foo
```

This allows Windows IDEs such as IntelliJ IDEA to open the project.

Likewise:

```text
Open folder
```

should use Explorer and the UNC path.

Command execution remains inside WSL.

---

# Diagnostics

Improve command errors while touching this area.

When a command cannot be executed, include at least:

```text
Runtime: Windows / WSL
Shell: PowerShell 7 / cmd / bash
Working directory
Executable involved
```

For example:

```text
Unable to start command.

Runtime: Windows
Shell: PowerShell 7
Executable: pwsh.exe
Project: C:\dev\foo

pwsh.exe was not found.
```

For WSL:

```text
Unable to start command.

Runtime: WSL
Distribution: Ubuntu
Path: /home/julie/dev/foo

The selected WSL distribution is not available.
```

Do not dump the complete environment or secrets into logs.

---

# Backward compatibility

This is mandatory.

An existing CodeDeck installation must continue to work unchanged.

Old projects without execution configuration resolve to:

```text
Native + existing platform default
```

Therefore on Windows:

```text
cmd.exe
```

remains the default.

Existing exported/imported configuration JSON must remain readable.

New fields should use Serde defaults where appropriate.

---

# Security requirements

Do not build WSL or PowerShell invocation by concatenating:

```text
distro
path
environment values
```

into command strings.

Use `std::process::Command` argument separation.

The project command itself remains intentionally shell code because CodeDeck is explicitly a command launcher.

Do not expose `.env` contents or inherited environment variables in diagnostics.

Do not automatically execute detected/imported commands.

These constraints are consistent with CodeDeck's existing contribution/security rules.

---

# Tests

Add unit tests around the command-building layer so most behavior can be verified without actually having WSL installed.

Required coverage:

```text
Legacy config -> existing behavior

Windows / CMD
  command constructed correctly

Windows / PowerShell 7
  uses pwsh.exe
  profile loading is NOT disabled

Windows / Windows PowerShell
  uses powershell.exe

WSL
  distro passed as independent argument
  Linux path passed correctly
  command passed correctly
  configured environment variables propagated

UNC -> WSL path conversion

Windows path -> WSL path conversion

WSL distro enumeration decoding

invalid/missing distro error

PowerShell missing error

WSL process stop -> Linux process group

old exported configuration still deserializes
```

Manual Windows test matrix:

```text
Windows project + cmd
Windows project + PowerShell 7
Windows project + fnm + pnpm
WSL project + pnpm
WSL project + Git
Start Vite in WSL
Stop Vite
Confirm no Node process remains
Open terminal
Open IDE
Open project folder
```

For the specific `fnm` regression:

```text
PowerShell profile initializes fnm
Node is NOT globally available to cmd.exe

CodeDeck configured for PowerShell 7
pnpm -v succeeds
pnpm dev succeeds
```

That is an explicit acceptance criterion.

---

# Suggested implementation order

I would make the work internally as three commits, and probably upstream it as **two PRs**, because CodeDeck explicitly asks for focused PRs.

**PR 1 — Configurable Windows command shell**

```text
refactor: introduce execution target abstraction
feat: add PowerShell command execution
feat: add command shell settings
```

No WSL yet.

Acceptance test: your Windows project using `fnm` successfully executes:

```text
pnpm -v
pnpm build
pnpm dev
```

from CodeDeck.

**PR 2 — WSL execution backend**

```text
feat: detect WSL distributions
feat: add WSL project execution target
feat: map WSL project paths
feat: manage WSL process groups
feat: run Git inside project execution environment
feat: open WSL terminals
```

This separation gives the maintainer a much easier diff to review.

---

## One architectural point I would insist on

Je demanderais explicitement à ton agent de **ne pas créer un `if wsl { ... }` partout**.

L'objectif doit être :

```text
                    ┌─ Windows / CMD
Project ─ Execution ├─ Windows / PowerShell
                    ├─ WSL / Ubuntu
                    └─ future: SSH / container / etc.
```

et les consumers demandent juste au runtime :

```text
spawn command
run program
stop process
resolve cwd
open terminal
```
