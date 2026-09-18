import type { AppSettings, CommandShellInfo, ExecutionTarget, NativeShell, Project } from "../types/models";

export function resolveNativeShell(
  project: Pick<Project, "commandShell">,
  settings: Pick<AppSettings, "commandShell">,
): NativeShell {
  if (project.commandShell && project.commandShell !== "inherit") return project.commandShell;
  return settings.commandShell ?? "platformDefault";
}

export function resolveExecutionTarget(
  project: Pick<Project, "commandShell">,
  settings: Pick<AppSettings, "commandShell">,
): ExecutionTarget {
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
