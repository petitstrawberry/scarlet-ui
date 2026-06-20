import { execFileSync, spawn, type ChildProcess } from "child_process";
import * as path from "path";
import { Disposable, EventEmitter, workspace } from "vscode";
import { JsonRpcClient } from "./client";
import { getServerCommand } from "./config";
import { METHODS, type InitializeResult, type ServerNotificationHandlers } from "./protocol";

export type ServerState = "stopped" | "starting" | "ready" | "building" | "error";

function isInstalledPreviewCommand(cmd: string): boolean {
  return path.basename(cmd) === "scarlet-ui-preview";
}

function buildSpawnEnv(cmd: string, onStderr: (line: string) => void): NodeJS.ProcessEnv {
  const env = { ...process.env };
  if (!isInstalledPreviewCommand(cmd)) {
    return env;
  }

  const dyldKey = process.platform === "darwin"
    ? "DYLD_FALLBACK_LIBRARY_PATH"
    : process.platform === "linux"
      ? "LD_LIBRARY_PATH"
      : null;

  if (!dyldKey) {
    return env;
  }

  try {
    const targetLibdir = execFileSync("rustc", ["--print", "target-libdir"], {
      encoding: "utf-8",
      env,
    }).trim();

    if (targetLibdir) {
      const existing = env[dyldKey];
      env[dyldKey] = existing ? `${targetLibdir}:${existing}` : targetLibdir;
      onStderr(
        `[server] using ${dyldKey}=${targetLibdir} for installed scarlet-ui-preview`,
      );
    }
  } catch {
    onStderr(
      "[server] warning: failed to resolve rustc target-libdir for installed scarlet-ui-preview",
    );
  }

  return env;
}

export class ScarletUiServer implements Disposable {
  private process: ChildProcess | null = null;
  private client: JsonRpcClient | null = null;
  private state: ServerState = "stopped";
  private manifestPath: string | null = null;
  private disposed = false;

  private readonly _onStateChanged = new EventEmitter<ServerState>();
  readonly onStateChanged = this._onStateChanged.event;

  constructor(
    private readonly notificationHandlers: ServerNotificationHandlers,
    private readonly onStderr: (line: string) => void,
  ) {}

  getState(): ServerState {
    return this.state;
  }

  getClient(): JsonRpcClient | null {
    return this.client;
  }

  getManifestPath(): string | null {
    return this.manifestPath;
  }

  private setState(state: ServerState): void {
    this.state = state;
    if (!this.disposed) {
      this._onStateChanged.fire(state);
    }
  }

  async start(manifestPath: string): Promise<InitializeResult> {
    if (this.process) throw new Error("Server already started");
    this.manifestPath = manifestPath;
    this.setState("starting");

    const baseCmd = getServerCommand();
    if (baseCmd.length === 0) throw new Error("scarlet-ui.serverCommand must not be empty");

    const args = [...baseCmd, "--manifest-path", manifestPath];
    const cmd = args[0];
    const cmdArgs = args.slice(1);
    const cwd = workspace.workspaceFolders?.[0]?.uri.fsPath;
    this.onStderr(
      `[server] starting: ${cmd} ${cmdArgs.join(" ")} (cwd: ${cwd ?? process.cwd()})`,
    );
    const env = buildSpawnEnv(cmd, this.onStderr);
    this.process = spawn(cmd, cmdArgs, { stdio: ["pipe", "pipe", "pipe"], cwd, env });

    this.process.once("error", (err: NodeJS.ErrnoException) => {
      const hint = err.code === "ENOENT"
        ? `"${cmd}" not found. Set "scarlet-ui.serverCommand" to the full binary path. If you use the installed binary, note that preview builds still require cargo/rustc on PATH.`
        : err.message;
      this.onStderr(`[server] failed to start: ${hint}`);
      this.setState("error");
      this.process = null;
      this.client = null;
    });

    if (this.process.stderr) {
      let stderrBuf = "";
      this.process.stderr.on("data", (chunk: Buffer) => {
        stderrBuf += chunk.toString("utf-8");
        let idx: number;
        while ((idx = stderrBuf.indexOf("\n")) >= 0) {
          const line = stderrBuf.slice(0, idx);
          stderrBuf = stderrBuf.slice(idx + 1);
          if (line.trim()) this.onStderr(line);
        }
      });
    }

    if (!this.process.stdin || !this.process.stdout) {
      throw new Error("Failed to open stdio pipes");
    }

    this.client = new JsonRpcClient(this.process.stdin, this.process.stdout);
    this.client.setNotificationHandlers(this.notificationHandlers);
    this.client.on("error", (err: Error) => this.onStderr(`[rpc] ${err.message}`));
    this.client.on("close", () => {
      this.setState("stopped");
      this.process = null;
      this.client = null;
    });

    this.process.on("exit", (code) => {
      if (this.state !== "stopped") {
        this.onStderr(`[server] process exited with code ${code}`);
      }
      this.setState("stopped");
      this.process = null;
      this.client = null;
    });

    try {
      const result = (await this.client.sendRequest(METHODS.INITIALIZE, {
        client: "vscode-scarlet-ui",
        protocolVersion: 1,
      })) as InitializeResult;
      this.setState("ready");
      return result;
    } catch (err) {
      this.setState("error");
      throw err;
    }
  }

  async stop(): Promise<void> {
    if (!this.client || !this.process) {
      this.dispose();
      return;
    }

    try {
      await this.client.sendRequest(METHODS.SHUTDOWN, {});
    } catch {
      // Force cleanup below if the server cannot answer shutdown.
    }
    this.dispose();
  }

  markBuilding(): void {
    if (this.state === "ready" || this.state === "error") this.setState("building");
  }

  markReady(): void {
    if (this.state === "building") this.setState("ready");
  }

  markError(): void {
    this.setState("error");
  }

  dispose(): void {
    this.client = null;
    if (this.process) {
      this.process.kill();
      this.process = null;
    }
    this.setState("stopped");
    if (!this.disposed) {
      this.disposed = true;
      this._onStateChanged.dispose();
    }
  }
}
