import * as vscode from "vscode";
import { resolveTargetManifest } from "./config";
import { scanPreviews, toPreviewDisplayName } from "./previewScanner";
import type { OutputChannelManager } from "./outputChannel";
import { METHODS, type InitializeResult, type PreviewDescriptor } from "./protocol";
import type { ScarletUiServer } from "./serverManager";
import type { StatusBarManager } from "./statusBar";

export interface ScarletUiExtensionContext {
  server: ScarletUiServer;
  statusBar: StatusBarManager;
  output: OutputChannelManager;
}

export function registerCommands(ctx: ScarletUiExtensionContext, subscriptions: vscode.Disposable[]): void {
  async function ensureServer(): Promise<boolean> {
    if (ctx.server.getClient()) return true;
    return startServer(false);
  }

  async function startServer(autoPreview: boolean): Promise<boolean> {
    const manifest = resolveTargetManifest();
    if (!manifest) {
      vscode.window.showErrorMessage(
        "ScarletUI: No package manifest found. Set \"scarlet-ui.targetManifest\" or open a workspace with a Cargo.toml.",
      );
      return false;
    }

    try {
      await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "Starting ScarletUI preview server…" },
        async () => {
          const result = await ctx.server.start(manifest);
          const init = result as InitializeResult;
          ctx.output.logEvent(`Server started — package: ${init.package}`);
          ctx.statusBar.setState("ready", init.package);
        },
      );
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showErrorMessage(`ScarletUI: Failed to start server — ${msg}`);
      ctx.statusBar.setState("error", msg);
      ctx.output.showLogs();
      return false;
    }

    if (autoPreview) {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "rust") {
        await previewCurrentFile();
      }
    }
    return true;
  }

  async function previewCurrentFile(args?: { preview?: string }): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== "rust") {
      vscode.window.showWarningMessage("ScarletUI: Open a Rust source file first.");
      return;
    }
    if (!(await ensureServer())) return;

    const client = ctx.server.getClient()!;

    let previewName: string | undefined;
    if (args?.preview) {
      previewName = toPreviewDisplayName(args.preview);
    } else {
      const detected = scanPreviews(editor.document.getText());
      if (detected.length > 1) {
        const items = detected.map((p) => ({
          label: toPreviewDisplayName(p.functionName),
          description: p.functionName,
        }));
        const picked = await vscode.window.showQuickPick(items, {
          title: "ScarletUI: Select Preview",
          placeHolder: `${detected.length} previews found in this file`,
        });
        if (!picked) return;
        previewName = picked.label;
      } else if (detected.length === 1) {
        previewName = toPreviewDisplayName(detected[0].functionName);
      } else {
        vscode.window.showInformationMessage("ScarletUI: No #[scarlet_ui::preview] functions found in the current file.");
        return;
      }
    }

    try {
      ctx.statusBar.setState("building");
      ctx.output.logEvent(`preview/open — source: ${editor.document.uri.fsPath}, preview: ${previewName ?? "(first available)"}`);
      const result = (await client.sendRequest(METHODS.PREVIEW_OPEN, {
        source: editor.document.uri.fsPath,
        preview: previewName,
      })) as { preview: PreviewDescriptor };
      ctx.output.logEvent(`Preview opened — ${result.preview.name}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showErrorMessage(`ScarletUI: preview/open failed — ${msg}`);
      ctx.statusBar.setState("error", msg);
    }
  }

  async function selectPreview(): Promise<void> {
    if (!(await ensureServer())) return;
    const client = ctx.server.getClient()!;

    try {
      const result = (await client.sendRequest(METHODS.PREVIEW_LIST, {})) as {
        previews: PreviewDescriptor[];
      };
      if (result.previews.length === 0) {
        vscode.window.showInformationMessage("ScarletUI: No previews available.");
        return;
      }

      const items = result.previews.map((p) => ({ label: p.name, description: p.id, preview: p }));
      const picked = await vscode.window.showQuickPick(items, {
        title: "ScarletUI: Select Preview",
        placeHolder: "Choose a preview to display",
      });
      if (!picked) return;

      try {
        await client.sendRequest(METHODS.PREVIEW_SWITCH_PREVIEW, { preview: picked.preview.name });
        ctx.output.logEvent(`Switched preview — ${picked.preview.name}`);
      } catch {
        await client.sendRequest(METHODS.PREVIEW_OPEN, { preview: picked.preview.name });
        ctx.output.logEvent(`Opened preview — ${picked.preview.name}`);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showErrorMessage(`ScarletUI: select preview failed — ${msg}`);
    }
  }

  async function rebuild(): Promise<void> {
    if (!(await ensureServer())) return;
    const client = ctx.server.getClient()!;

    try {
      ctx.statusBar.setState("building");
      ctx.output.logEvent("preview/rebuild");
      await client.sendRequest(METHODS.PREVIEW_REBUILD, {});
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showErrorMessage(`ScarletUI: rebuild failed — ${msg}`);
    }
  }

  async function closeWindow(): Promise<void> {
    const client = ctx.server.getClient();
    if (!client) return;

    try {
      await client.sendRequest(METHODS.PREVIEW_CLOSE_WINDOW, {});
      ctx.output.logEvent("Preview window closed");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showWarningMessage(`ScarletUI: close window failed — ${msg}`);
    }
  }

  async function stopServer(): Promise<void> {
    try {
      await ctx.server.stop();
      ctx.output.logEvent("Server stopped");
      ctx.statusBar.setState("stopped");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      vscode.window.showErrorMessage(`ScarletUI: stop failed — ${msg}`);
    }
  }

  async function statusMenu(): Promise<void> {
    const state = ctx.server.getState();
    const cfg = vscode.workspace.getConfiguration("scarlet-ui");
    const autoFollow = cfg.get<boolean>("autoFollowActiveEditor", true);

    type Action = { label: string; run: () => Promise<unknown> };
    const actions: Action[] = [];

    if (state === "stopped" || state === "error") {
      actions.push({ label: "$(play) Start Server", run: () => startServer(true) });
    } else {
      actions.push({ label: "$(eye) Preview Current File", run: previewCurrentFile });
      actions.push({ label: "$(list-selection) Select Preview", run: selectPreview });
      actions.push({ label: "$(refresh) Rebuild", run: rebuild });
      actions.push({ label: "$(close) Close Window", run: closeWindow });
      actions.push({ label: "$(stop) Stop Server", run: stopServer });
    }

    actions.push({
      label: autoFollow
        ? "$(circle-slash) Disable Auto-Follow"
        : "$(circle-check) Enable Auto-Follow",
      run: async () => { await cfg.update("autoFollowActiveEditor", !autoFollow); },
    });
    actions.push({ label: "$(output) Show Logs", run: async () => ctx.output.showLogs() });

    const picked = await vscode.window.showQuickPick(
      actions.map((a) => ({ label: a.label })),
      { title: "ScarletUI", placeHolder: `Server: ${state}` },
    );
    if (!picked) return;
    const action = actions.find((a) => a.label === picked.label);
    await action?.run();
  }

  subscriptions.push(
    vscode.commands.registerCommand("scarlet-ui.startServer", () => startServer(true)),
    vscode.commands.registerCommand("scarlet-ui.previewCurrentFile", previewCurrentFile),
    vscode.commands.registerCommand("scarlet-ui.selectPreview", selectPreview),
    vscode.commands.registerCommand("scarlet-ui.rebuild", rebuild),
    vscode.commands.registerCommand("scarlet-ui.closeWindow", closeWindow),
    vscode.commands.registerCommand("scarlet-ui.stopServer", stopServer),
    vscode.commands.registerCommand("scarlet-ui.statusMenu", statusMenu),
  );
}
