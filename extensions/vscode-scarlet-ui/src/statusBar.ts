import * as vscode from "vscode";
import type { ServerState } from "./serverManager";

export class StatusBarManager implements vscode.Disposable {
  private item: vscode.StatusBarItem;

  constructor() {
    this.item = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Right,
      50,
    );
    this.item.command = "scarlet-ui.statusMenu";
    this.setState("stopped");
    this.item.show();
  }

  setState(state: ServerState, detail?: string): void {
    switch (state) {
      case "stopped":
        this.item.text = "$(circle-outline) ScarletUI";
        this.item.tooltip = "ScarletUI server stopped — click for menu";
        break;
      case "starting":
        this.item.text = "$(sync~spin) ScarletUI";
        this.item.tooltip = "Starting…";
        break;
      case "ready":
        this.item.text = "$(checkmark) ScarletUI";
        this.item.tooltip = detail ? `Ready — ${detail}` : "Ready — click for menu";
        break;
      case "building":
        this.item.text = "$(sync~spin) ScarletUI";
        this.item.tooltip = "Building…";
        break;
      case "error":
        this.item.text = "$(error) ScarletUI";
        this.item.tooltip = detail ?? "Error — click for menu";
        break;
    }
  }

  dispose(): void {
    this.item.dispose();
  }
}
