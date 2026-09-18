import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  BuiltInProjectTemplateId,
  CommandShellInfo,
  CreatedProject,
  EditorSuggestion,
  ExecutionTarget,
  GitConflictContent,
  GitRepositoryStatus,
  ProcessExitEvent,
  ProcessOutputEvent,
  ProjectCandidate,
  ProjectInspection,
  WslLocation,
} from "../types/models";

export const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const setApplicationLanguage = (language: "de" | "en") =>
  call<void>("set_application_language", { language });

const runtimeText = (german: string, english: string) =>
  typeof document !== "undefined" && document.documentElement.lang === "en" ? english : german;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error(runtimeText("Diese Aktion ist nur in der Desktop-App verfügbar.", "This action is only available in the desktop app."));
  }
  return invoke<T>(command, args);
}

export async function chooseDirectory(defaultPath?: string) {
  if (!isTauri()) return null;
  const selected = await open({
    directory: true,
    multiple: false,
    defaultPath: defaultPath || undefined,
    title: runtimeText("Projektordner auswählen", "Select project folder"),
  });
  return typeof selected === "string" ? selected : null;
}

export async function chooseConfigFile() {
  if (!isTauri()) return null;
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: runtimeText("Code Deck Konfiguration", "Code Deck configuration"), extensions: ["json"] }],
    title: runtimeText("Code Deck Konfiguration importieren", "Import Code Deck configuration"),
  });
  return typeof selected === "string" ? selected : null;
}

export async function chooseExportPath() {
  if (!isTauri()) return null;
  return save({
    defaultPath: `code-deck-backup-${new Date().toISOString().slice(0, 10)}.json`,
    filters: [{ name: "JSON", extensions: ["json"] }],
    title: runtimeText("Code Deck Konfiguration exportieren", "Export Code Deck configuration"),
  });
}


export const createProjectFromTemplate = (
  parentPath: string,
  projectName: string,
  templateId: BuiltInProjectTemplateId | "custom",
  customTemplatePath: string | undefined,
  initGit: boolean,
  javaPackageBase?: string,
) =>
  call<CreatedProject>("create_project_from_template", {
    parentPath,
    projectName,
    templateId,
    customTemplatePath: customTemplatePath || null,
    initGit,
    javaPackageBase: javaPackageBase || null,
  });

export const inspectProject = (path: string, executionTarget: ExecutionTarget) =>
  call<ProjectInspection>("inspect_project", { path, executionTarget });

export const detectWslDistros = () =>
  call<string[]>("detect_wsl_distros");

export const resolveWslLocation = (path: string, distro?: string) =>
  call<WslLocation>("resolve_wsl_location", { path, distro: distro || null });

export const cloneRepository = (
  repositoryUrl: string,
  parentPath: string,
  directoryName?: string,
  branch?: string,
  shallow = false,
) =>
  call<CreatedProject>("clone_repository", {
    repositoryUrl,
    parentPath,
    directoryName: directoryName || null,
    branch: branch || null,
    shallow,
  });

export const initializeGitRepository = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<void>("git_init_repository", { projectPath, executionTarget });

export const getGitStatus = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<GitRepositoryStatus>("git_status", { projectPath, executionTarget });

export const getGitBranches = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<string[]>("git_branches", { projectPath, executionTarget });

export const getGitDiff = (
  projectPath: string,
  filePath: string,
  state: { staged: boolean; unstaged: boolean; untracked: boolean },
  executionTarget: ExecutionTarget,
) => call<string>("git_diff", { projectPath, filePath, ...state, executionTarget });

export const gitStageFiles = (projectPath: string, paths: string[], executionTarget: ExecutionTarget) =>
  call<void>("git_stage", { projectPath, paths, executionTarget });

export const gitUnstageFiles = (projectPath: string, paths: string[], executionTarget: ExecutionTarget) =>
  call<void>("git_unstage", { projectPath, paths, executionTarget });

export const gitCommit = (projectPath: string, message: string, executionTarget: ExecutionTarget) =>
  call<void>("git_commit", { projectPath, message, executionTarget });

export const gitCheckoutBranch = (projectPath: string, branch: string, executionTarget: ExecutionTarget) =>
  call<void>("git_checkout_branch", { projectPath, branch, executionTarget });

export const gitCreateBranch = (projectPath: string, branch: string, executionTarget: ExecutionTarget) =>
  call<void>("git_create_branch", { projectPath, branch, executionTarget });

export const gitRemoteAction = (projectPath: string, action: "fetch" | "pull" | "push", executionTarget: ExecutionTarget) =>
  call<string>("git_remote_action", { projectPath, action, executionTarget });

export const getGitRemoteUrl = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<string | null>("git_remote_url", { projectPath, executionTarget });

export const getGitConflict = (projectPath: string, filePath: string, executionTarget: ExecutionTarget) =>
  call<GitConflictContent>("git_conflict_content", { projectPath, filePath, executionTarget });

export const resolveGitConflict = (projectPath: string, filePath: string, contents: string, executionTarget: ExecutionTarget) =>
  call<void>("git_resolve_conflict", { projectPath, filePath, contents, executionTarget });

export const continueGitOperation = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<void>("git_continue_operation", { projectPath, executionTarget });

export const abortGitOperation = (projectPath: string, executionTarget: ExecutionTarget) =>
  call<void>("git_abort_operation", { projectPath, executionTarget });

export const scanProjects = (path: string) =>
  call<ProjectCandidate[]>("scan_projects", { path });

export const detectEditors = () =>
  call<EditorSuggestion[]>("detect_editors");

export const getDesktopDirectory = () =>
  call<string>("get_desktop_directory");

export const launchTemplate = (
  commandTemplate: string,
  projectPath: string,
  projectName: string,
) =>
  call<void>("launch_template", {
    commandTemplate,
    projectPath,
    projectName,
  });

export const openTerminal = (
  projectPath: string,
  terminalCommand: string,
  executionTarget?: ExecutionTarget,
) =>
  call<void>("open_terminal", { projectPath, terminalCommand, executionTarget: executionTarget || null });

export const detectCommandShells = () =>
  call<CommandShellInfo[]>("detect_command_shells");

export const openTarget = (target: string) =>
  call<void>("open_target", { target });

export const startProcess = (
  runId: string,
  projectPath: string,
  command: string,
  workingDir?: string,
  env: Record<string, string> = {},
  label = command,
  notifyOnExit = false,
  executionTarget?: ExecutionTarget,
) =>
  call<{ pid: number }>("start_process", {
    runId,
    projectPath,
    command,
    workingDir: workingDir || null,
    env,
    label,
    notifyOnExit,
    executionTarget: executionTarget || null,
  });

export const stopProcess = (runId: string, pid: number) =>
  call<void>("stop_process", { runId, pid });

export const readTextFile = (path: string) =>
  call<string>("read_text_file", { path });

export const writeTextFile = (path: string, contents: string) =>
  call<void>("write_text_file", { path, contents });

export function onProcessOutput(
  handler: (event: ProcessOutputEvent) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => undefined);
  return listen<ProcessOutputEvent>("code-deck://process-output", (event) =>
    handler(event.payload),
  );
}

export function onProcessExit(
  handler: (event: ProcessExitEvent) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => undefined);
  return listen<ProcessExitEvent>("code-deck://process-exit", (event) =>
    handler(event.payload),
  );
}
