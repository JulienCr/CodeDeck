import type { AppSettings, CommandShellInfo, ExecutionTarget, NativeShell, Project } from "../types/models";

export function resolveNativeShell(
  project: Pick<Project, "commandShell">,
  settings: Pick<AppSettings, "commandShell">,
): NativeShell {
  if (project.commandShell && project.commandShell !== "inherit") return project.commandShell;
  return settings.commandShell ?? "platformDefault";
}

export function resolveExecutionTarget(
  project: Pick<Project, "commandShell" | "executionRuntime" | "wslDistro" | "wslPath">,
  settings: Pick<AppSettings, "commandShell">,
): ExecutionTarget {
  if (project.executionRuntime === "wsl") {
    return { type: "wsl", distro: project.wslDistro ?? "", linuxPath: project.wslPath ?? "" };
  }
  return { type: "native", shell: resolveNativeShell(project, settings) };
}

export function nativeShellLabel(t: (de: string, en: string) => string, shell: NativeShell): string {
  if (shell === "platformDefault") return t("Plattform-Standard", "Platform default");
  if (shell === "cmd") return t("Eingabeaufforderung", "Command Prompt");
  if (shell === "powerShell7") return "PowerShell 7";
  return "Windows PowerShell";
}

export function commandShellOptionLabel(t: (de: string, en: string) => string, shell: CommandShellInfo): string {
  const name = nativeShellLabel(t, shell.id);
  return shell.available ? name : `${name} (${t("nicht gefunden", "not found")})`;
}

// Used to inspect a not-yet-added project, which has no runtime settings yet.
export function defaultExecutionTarget(): ExecutionTarget {
  return { type: "native", shell: "platformDefault" };
}

export function executionTargetLabel(t: (de: string, en: string) => string, target: ExecutionTarget): string {
  if (target.type === "wsl") {
    return target.distro ? `WSL · ${target.distro}` : "WSL";
  }
  return `Windows · ${nativeShellLabel(t, target.shell)}`;
}
