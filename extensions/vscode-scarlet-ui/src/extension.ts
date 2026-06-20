import * as vscode from "vscode";
import { ScarletUiCodeLensProvider } from "./codeLensProvider";
import { getAutoFollow } from "./config";
import { registerCommands, type ScarletUiExtensionContext } from "./commands";
import { OutputChannelManager } from "./outputChannel";
import { scanPreviews, toPreviewDisplayName } from "./previewScanner";
import { METHODS, type ServerNotificationHandlers } from "./protocol";
import { ScarletUiServer } from "./serverManager";
import { StatusBarManager } from "./statusBar";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = new OutputChannelManager();
  const statusBar = new StatusBarManager();

  const handlers: ServerNotificationHandlers = {
    [METHODS.BUILD_STARTED]: (params) => {
      statusBar.setState("building");
      output.logEvent(`Build started — source: ${params.source ?? "(default)"}`);
    },
    [METHODS.BUILD_FINISHED]: (params) => {
      statusBar.setState("ready", `${params.previews.length} preview(s)`);
      output.logEvent(`Build finished — ${params.previews.length} preview(s)`);
    },
    [METHODS.RELOAD_FINISHED]: (params) => {
      statusBar.setState("ready", params.preview.name);
      output.logEvent(`Reload finished — ${params.preview.name}`);
    },
    [METHODS.BUILD_FAILED]: (params) => {
      statusBar.setState("error", params.message);
      output.logEvent(`Build failed — ${params.message}`);
      output.showLogs();
    },
    [METHODS.WINDOW_CLOSED]: () => {
      statusBar.setState("ready", "window closed");
      output.logEvent("Preview window closed");
    },
  };

  const server = new ScarletUiServer(handlers, (line) => output.logStderr(line));
  server.onStateChanged((state) => {
    if (state === "stopped" || state === "starting") {
      statusBar.setState(state);
    }
  });

  const ctx: ScarletUiExtensionContext = { server, statusBar, output };

  registerCommands(ctx, context.subscriptions);

  context.subscriptions.push(
    vscode.languages.registerCodeLensProvider(
      { scheme: "file", pattern: "**/*.rs" },
      new ScarletUiCodeLensProvider(),
    ),
  );

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(async (editor) => {
      if (!editor || !editor.document.uri.fsPath.endsWith(".rs")) return;
      const client = server.getClient();
      if (!client || client.isClosed()) return;

      const filePath = editor.document.uri.fsPath;
      client.sendNotification(METHODS.DID_CHANGE_ACTIVE_FILE, { path: filePath });

      if (!getAutoFollow()) return;

      const detected = scanPreviews(editor.document.getText());
      if (detected.length === 0) return;

      const previewName = toPreviewDisplayName(detected[0].functionName);
      try {
        await client.sendRequest(METHODS.PREVIEW_SWITCH_SOURCE, {
          source: filePath,
          preview: previewName,
        });
        output.logEvent(`Auto-switched source to ${filePath} — ${previewName}`);
      } catch {}
    }),
  );

  context.subscriptions.push(output, statusBar, server);
}

export function deactivate(): void {}
