export type RequestId = number | string | null;

export interface ErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export const PARSE_ERROR = -32700;
export const INVALID_REQUEST = -32600;
export const METHOD_NOT_FOUND = -32601;
export const INVALID_PARAMS = -32602;
export const BUILD_FAILED = -32001;
export const PREVIEW_NOT_FOUND = -32002;
export const PREVIEW_LIBRARY_LOAD_FAILED = -32003;
export const PREVIEW_HOST_WINDOW_ERROR = -32004;

export interface PreviewDescriptor {
  id: string;
  name: string;
}

export interface InitializeResult {
  protocolVersion: number;
  package: string;
  capabilities: {
    switchSource: boolean;
    switchPreview: boolean;
    watch: boolean;
  };
}

export interface InitializeParams {
  client?: string;
  protocolVersion?: number;
}

export interface PreviewListParams {
  source?: string;
}

export interface PreviewOpenParams {
  source?: string;
  preview?: string | null;
}

export interface SwitchSourceParams {
  source: string;
  preview?: string | null;
}

export interface SwitchPreviewParams {
  preview: string;
}

export interface BuildStartedParams {
  source: string | null;
}

export interface BuildFinishedParams {
  previews: PreviewDescriptor[];
}

export interface ReloadFinishedParams {
  preview: PreviewDescriptor;
}

export interface BuildFailedParams {
  message: string;
}

export interface DidChangeActiveFileParams {
  path: string;
}

export const METHODS = {
  INITIALIZE: "initialize",
  PREVIEW_LIST: "preview/list",
  PREVIEW_OPEN: "preview/open",
  PREVIEW_SWITCH_SOURCE: "preview/switchSource",
  PREVIEW_SWITCH_PREVIEW: "preview/switchPreview",
  PREVIEW_REBUILD: "preview/rebuild",
  PREVIEW_CLOSE_WINDOW: "preview/closeWindow",
  SHUTDOWN: "shutdown",
  BUILD_STARTED: "preview/buildStarted",
  BUILD_FINISHED: "preview/buildFinished",
  RELOAD_FINISHED: "preview/reloadFinished",
  BUILD_FAILED: "preview/buildFailed",
  WINDOW_CLOSED: "preview/windowClosed",
  DID_CHANGE_ACTIVE_FILE: "workspace/didChangeActiveFile",
} as const;

export interface ServerNotificationHandlers {
  [METHODS.BUILD_STARTED]?: (params: BuildStartedParams) => void;
  [METHODS.BUILD_FINISHED]?: (params: BuildFinishedParams) => void;
  [METHODS.RELOAD_FINISHED]?: (params: ReloadFinishedParams) => void;
  [METHODS.BUILD_FAILED]?: (params: BuildFailedParams) => void;
  [METHODS.WINDOW_CLOSED]?: () => void;
}
