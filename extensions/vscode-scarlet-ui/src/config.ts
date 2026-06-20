import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

export function getServerCommand(): string[] {
  return vscode.workspace.getConfiguration("scarlet-ui").get<string[]>("serverCommand", [
    "scarlet-ui-preview",
    "serve",
  ]);
}

export function getTargetManifest(): string | null {
  return vscode.workspace.getConfiguration("scarlet-ui").get<string | null>("targetManifest", null);
}

export function getAutoFollow(): boolean {
  return vscode.workspace.getConfiguration("scarlet-ui").get<boolean>("autoFollowActiveEditor", false);
}

/**
 * Resolve the target manifest path.
 *
 * 1. If `scarlet-ui.targetManifest` is configured, use it.
 * 2. If the workspace root Cargo.toml has a [package] section, use it.
 * 3. If the root is a virtual workspace (no [package]), try
 *    `examples/preview-demo/Cargo.toml` as a sensible default.
 * 4. Otherwise return null (the caller should show an error).
 */
export function resolveTargetManifest(): string | null {
  const configured = getTargetManifest();
  if (configured) return configured;

  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) return null;

  const rootManifest = path.join(folder.uri.fsPath, "Cargo.toml");

  try {
    const content = fs.readFileSync(rootManifest, "utf-8");
    if (/\[package\]/.test(content)) {
      return rootManifest;
    }
    for (const candidate of ["examples/preview-demo/Cargo.toml"]) {
      const candidatePath = path.join(folder.uri.fsPath, candidate);
      try {
        fs.accessSync(candidatePath);
        return candidatePath;
      } catch {
        continue;
      }
    }
  } catch {
    return rootManifest;
  }

  return null;
}
