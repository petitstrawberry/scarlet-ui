import * as vscode from "vscode";
import { scanPreviews } from "./previewScanner";

export class ScarletUiCodeLensProvider implements vscode.CodeLensProvider {
  provideCodeLenses(document: vscode.TextDocument): vscode.CodeLens[] {
    const previews = scanPreviews(document.getText());
    return previews.map(
      (p) =>
        new vscode.CodeLens(new vscode.Range(p.line, 0, p.line, 0), {
          title: "▶ Preview",
          command: "scarlet-ui.previewCurrentFile",
          arguments: [{ preview: p.functionName }],
        }),
    );
  }
}
