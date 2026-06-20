import { EventEmitter } from "events";
import type { Readable, Writable } from "stream";
import type { ErrorObject, ServerNotificationHandlers } from "./protocol";

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
}

export class JsonRpcClient extends EventEmitter {
  private nextId = 1;
  private pending = new Map<number, PendingRequest>();
  private buffer = "";
  private closed = false;

  constructor(
    private readonly stdin: Writable,
    private readonly stdout: Readable,
  ) {
    super();
    this.stdout.on("data", (chunk: Buffer) => this.onData(chunk));
    this.stdout.on("close", () => this.onClose());
    this.stdout.on("error", (err) => this.emit("error", err));
  }

  private onData(chunk: Buffer): void {
    this.buffer += chunk.toString("utf-8");
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, idx);
      this.buffer = this.buffer.slice(idx + 1);
      if (line.trim()) {
        this.handleMessage(line);
      }
    }
  }

  private handleMessage(raw: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }

    if (!this.isRecord(msg)) return;

    if ("result" in msg || "error" in msg) {
      const id = typeof msg.id === "number" ? msg.id : null;
      if (id !== null && this.pending.has(id)) {
        const pending = this.pending.get(id)!;
        this.pending.delete(id);
        if (msg.error) {
          pending.reject(this.toError(msg.error as ErrorObject));
        } else {
          pending.resolve(msg.result);
        }
      }
    } else if (typeof msg.method === "string") {
      this.emit("notification", msg.method, msg.params);
    }
  }

  private isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null;
  }

  private toError(errorObj: ErrorObject): Error {
    const err = new Error(errorObj.message);
    (err as Error & { code?: number }).code = errorObj.code;
    (err as Error & { data?: unknown }).data = errorObj.data;
    return err;
  }

  private onClose(): void {
    if (this.closed) return;
    this.closed = true;
    for (const [, pending] of this.pending) {
      pending.reject(new Error("JSON-RPC connection closed"));
    }
    this.pending.clear();
    this.emit("close");
  }

  sendRequest(method: string, params?: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      if (this.closed) {
        reject(new Error("JSON-RPC connection closed"));
        return;
      }

      const id = this.nextId++;
      this.pending.set(id, { resolve, reject });
      const msg = JSON.stringify({ jsonrpc: "2.0", id, method, params: params ?? {} });
      this.stdin.write(`${msg}\n`, (err) => {
        if (err) {
          this.pending.delete(id);
          reject(err);
        }
      });
    });
  }

  sendNotification(method: string, params?: unknown): void {
    if (this.closed) return;
    const msg = JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {} });
    this.stdin.write(`${msg}\n`);
  }

  setNotificationHandlers(handlers: ServerNotificationHandlers): void {
    this.on("notification", (method: string, params: unknown) => {
      const handler = (handlers as Record<string, ((params: unknown) => void) | undefined>)[method];
      if (handler) handler(params);
    });
  }

  isClosed(): boolean {
    return this.closed;
  }
}
