# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

CodeDeck is a Tauri 2 desktop app: a React 19 + TypeScript frontend (`src/`) over a Rust backend (`src-tauri/`) that launches IDEs, runs project commands and drives Git on the user's machine.

## Validation commands

```bash
pnpm install --frozen-lockfile
pnpm tauri:dev          # full desktop app
pnpm dev                # frontend only; every native call throws (see tauri.ts)
pnpm build              # tsc + vite build — the only TS gate, there is no ESLint/Prettier here
pnpm tauri:build        # bundles to src-tauri/target/release/bundle/
pnpm version:check      # the 4 version files must agree
```

Rust checks, exactly as CI runs them:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked
cargo test  --manifest-path src-tauri/Cargo.toml -- renders_untracked_text_as_added_lines   # a single test
```

Pre-PR gate: `pnpm build` plus the three cargo commands. There is no frontend test runner; the only automated tests are in `src-tauri/src/commands/git.rs` and `src-tauri/src/platform/launchers.rs`. CI runs clippy and the tests on ubuntu, windows and macos, so a change that only compiles on one OS fails there and not locally.

## Architecture

### Frontend ⇄ backend seam

- `src/app/App.tsx` is *the* component: it owns the whole `AppData` state and every action, then prop-drills into `src/features/<name>/`. A new feature is a component there that receives callbacks from `App.tsx`, not its own store.
- `src/shared/lib/tauri.ts` is the only place `invoke()` is called — one typed wrapper per command. **Adding a native capability is three edits**: the Rust fn, the `invoke_handler![]` list in `src-tauri/src/lib.rs`, and a wrapper in `tauri.ts`. Miss the middle one and the call fails only at runtime.
- Its `call()` helper throws when `__TAURI_INTERNALS__` is absent, which is why `pnpm dev` renders the UI but does nothing native.
- Arguments are camelCase from JS, snake_case in the Rust signature; Tauri converts. Every command returns `Result<T, String>` and that `String` is shown to the user verbatim.
- Process output is not a return value: `start_process` streams `code-deck://process-output` / `code-deck://process-exit` events, consumed through `onProcessOutput` / `onProcessExit`.
- The frontend never assembles a shell string for the OS. It passes a template or a command plus its context, and Rust decides how to spawn it.

### Where the execution logic lives

Keeping each concern in its own module is what keeps the per-OS branching contained:

| Module | Owns |
|---|---|
| `src-tauri/src/commands/` | `#[tauri::command]` entry points only. Validate, delegate, map the error to a `String`. No spawning. |
| `src-tauri/src/platform/launchers.rs` | How a launched (IDE, terminal) child process is *built*: `shell_command()`, `hide_console_window()`, template splitting, IDE and terminal detection per OS. |
| `src-tauri/src/platform/execution.rs` | How a project command's shell is chosen and its `Command` assembled: `ExecutionTarget`/`NativeShell`, `build_execution_command()`, Windows shell discovery. |
| `src-tauri/src/process/` | How a long-running process is *run*: spawn, stdout/stderr reader threads, event emission, stop. |
| `src-tauri/src/git/` | `run_git()` and the porcelain parser. Everything git-shaped goes through here. |
| `src-tauri/src/projects/` | Tech inspection, template scaffolding, and the name/path sanitizers in `validation.rs`. |

A command that spawns something itself, or a feature module that reimplements `run_git`, is in the wrong place.

### Persistence and config backward compatibility

- No database. Everything is one `localStorage` blob under `code-deck-data-v1`, written by a 200 ms debounced effect in `App.tsx`.
- `normalizeData()` in `src/shared/lib/storage.ts` is the migration layer — every load *and* every import goes through it.
- **Never break an existing config.** A user upgrading from any earlier version must land on a working app with their projects intact. Concretely: a new `AppData` field is added to `createDefaultData()` *and* defaulted in `normalizeData()`; a renamed field keeps reading the old name (`editor.path` and `action.targetId` are still honoured); an obsolete field is ignored, never fatal. Bumping `AppData["version"]` or dropping unknown data is not a shortcut around this.
- `normalizeData(input, /* imported */ true)` enforces the import security contract: commands become untrusted, `githubToken` is dropped, `autoStartOnAppLaunch` is forced false. Never bypass it for imported config.
- `read_text_file` / `write_text_file` exist only for config import/export, not for app state.

### i18n — German first

- UI strings: `const { t } = useI18n()` then `t("Deutsch", "English")`. No bare literals in JSX.
- `tauri.ts` cannot use the React context, so it reads `document.documentElement.lang` (set by an effect in `App.tsx`) to pick its language.
- Rust error strings are German only and surface as-is; match that rather than mixing languages in one file.
- `Workspace` in the code is "Launch set" in the UI and docs — same thing.

### Styling

Plain CSS, no Tailwind or CSS-in-JS: `src/styles.css` (global, ~4.5k lines) plus `src/features/github/github.css`. Classes are BEM-ish (`app-loading__header`); theming is CSS variables on `:root` overridden by `:root[data-theme="light"]`.

## Cross-platform rules

Windows, macOS and Linux are all first-class. A feature that works on one and silently no-ops on another is a bug, not a limitation.

### `cfg(target_os = ...)` conventions

- Branch on the **capability**, not at the call site: one `pub(crate) fn` per platform, same name and same signature, each behind its own `#[cfg]`, plus a fallback arm so the module still compiles everywhere. `hide_console_window`, `detect_platform_editors` and `send_system_notification` are the models to copy. Callers stay `cfg`-free.
- Use `#[cfg(all(unix, not(target_os = "macos")))]` for the Linux arm, never a bare `#[cfg(unix)]` — macOS is unix and would take the wrong branch.
- Every `#[cfg]`-gated function needs its counterpart arm for all other targets, even if the body is empty and the parameters are `_`-prefixed. A missing arm is a compile error on a platform you are not building.
- Gate the `use` statements too: an import used by only one arm is an `unused_imports` warning elsewhere, and clippy runs with `-D warnings` on all three OSes.
- Platform `cfg` belongs in `platform/`, with the few thin spots in `process/` and `git/`. A `cfg` creeping into `commands/` or a feature module means the abstraction is missing.

### Per-OS traps

- **Windows**: every spawned `Command` needs `CREATE_NO_WINDOW`, via `platform::launchers::hide_console_window(&mut cmd)` or by constructing it with `shell_command()`. Forgetting it flashes a console window at the user. `shell_command()` also prepends `chcp 65001` and sets `PYTHONUTF8` so child output arrives as UTF-8.
- **Linux**: user commands must go through `shell_command()`, which strips the AppImage's injected environment (`sanitize_appimage_environment`) and puts the child in its own process group. A raw `Command` leaks the bundle's `LD_LIBRARY_PATH` into the child. Flatpak editors launch through `flatpak run`, so an absolute-path check alone will not find them.
- **macOS**: editors are `.app` bundles launched through `open`, not executables on `PATH`.
- **Git**: use `git::repository::run_git()`, never a raw `Command::new("git")` — it sets `GIT_TERMINAL_PROMPT=0` and `GIT_EDITOR=true` so git can never block waiting on a prompt.
- Any repo-relative path arriving from the frontend goes through `safe_repo_path()`, which rejects absolute and `..` paths and canonicalizes against the repo root.
- Launch templates interpolate `{projectPath}` and `{projectName}` through `fill_template`, which escapes quotes.

### One execution abstraction, not scattered conditionals

When execution has to reach somewhere other than the local shell — WSL, a container, a remote host — it goes **into the spawn layer** (`shell_command()`, `launch_parts()`, `process::manager`) as one more way of building a `Command`. Do not sprinkle `if wsl` or `if is_windows` checks through `commands/`, the git module, the project templates or the React components. A dozen call sites each deciding for themselves is how path translation, quoting and environment end up disagreeing, and how each new site silently misses the rule the others already learned.

The practical test: adding a new execution target should touch the spawn layer and the setting that selects it, and nothing else.

### Tests expected for Windows / PowerShell / WSL

Anything that assembles a command line is pure and must be unit-tested without spawning — that is the reason `fill_template`, `split_command_template`, `linux_terminal_arguments` and any path translation stay separate functions. Cover at minimum:

- argument splitting and quoting for a path containing spaces, and for a path containing a quote;
- the Windows `cmd.exe /D /S /C` form and the PowerShell argument list (`-NoLogo -NonInteractive -Command`) — assert the built argv, never run it, and assert `-NoProfile` is **absent**: that negative check is the regression guard, since it is the flag a future contributor will helpfully add back;
- Windows ↔ WSL path translation both ways, including a drive letter, a UNC path, and a path that must be left alone;
- one test per branch of any new `#[cfg]` fork, gated with the same `#[cfg]` as the code it covers — `#[cfg(all(test, unix, not(target_os = "macos")))]` in `launchers.rs` is the existing pattern.

CI runs `cargo test` on all three OSes, so a `cfg`-gated test does get executed, on its own platform only.

## Secrets and logs

- Never log, emit or persist a full environment. `start_process` takes an explicit `env` map; that map is what runs, not what gets echoed into the process log or a toast.
- Never read, display, diff or attach `.env`, `.env.*`, key files or credential files on the user's behalf. The Git panel shows what the user selects; nothing auto-expands a sensitive file.
- `githubToken` lives in the config blob and goes out only as an `Authorization` header to `api.github.com`. It is stripped on export and import, and must never reach a log line, an error message, an event payload or a command line.
- Error strings are shown to the user verbatim — keep tokens, absolute home paths and command environments out of them.

## Releasing

The version lives in four files (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`) and CI fails when they disagree. **Never hand-edit them** — `pnpm release <version>` is the only supported path: it bumps all four, regenerates `CHANGELOG.md` and `RELEASE_NOTES_v<version>.md` from the commits since the last tag, runs the full check suite, commits and tags. Full flow in `docs/releasing.md`.

Those release notes are generated from commit subjects (`feat:` → Added, `fix:` → Fixed, `security:` → Security, everything else → Changed), and the release workflow refuses a tag whose notes file is missing or under 120 characters. Conventional commit prefixes are load-bearing here, not cosmetic.

## Conventions

- Branches: `feature/`, `fix/`, `docs/`, `refactor/`, `chore/`.
- Adding a Tauri plugin also means adding its permission to `src-tauri/capabilities/default.json`, or the call fails at runtime.
- Commands run only on an explicit user action, and imported commands require confirmation before their first run — see the "Project rules" section of `CONTRIBUTING.md`.
