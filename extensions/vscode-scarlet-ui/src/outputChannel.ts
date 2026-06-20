import * as vscode from "vscode";

export class OutputChannelManager implements vscode.Disposable {
  readonly events: vscode.OutputChannel;
  readonly logs: vscode.OutputChannel;

  constructor() {
    this.events = vscode.window.createOutputChannel("ScarletUI");
    this.logs = vscode.window.createOutputChannel("ScarletUI Logs");
  }

  logEvent(message: string): void {
    const ts = new Date().toISOString().slice(11, 19);
    this.events.appendLine(`[${ts}] ${message}`);
  }

  logStderr(line: string): void {
    this.logs.appendLine(line);
  }

  showEvents(): void {
    this.events.show(true);
  }

  showLogs(): void {
    this.logs.show(true);
  }

  dispose(): void {
    this.events.dispose();
    this.logs.dispose();
  }
}
