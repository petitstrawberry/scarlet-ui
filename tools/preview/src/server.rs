use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime};

use scarlet_ui::WinitBackend;
use scarlet_ui::platform::PlatformBackend;
use scarlet_ui::preview::{LoadedPreviewLibrary, PreviewHost};
use serde_json::{Value, json};

use crate::build::{CargoStdout, PreviewProject, build_and_load, prepare_project};
use crate::cli::ServeArgs;
use crate::rpc::{
    self, ErrorObject, Incoming, Notification, Params, PreviewDescriptorJson, Request, Response,
};
use crate::watch::latest_source_mtime;

static EMPTY_PARAMS: LazyLock<serde_json::Map<String, Value>> = LazyLock::new(serde_json::Map::new);

pub fn serve(args: ServeArgs) -> ExitCode {
    match PreviewServer::new(args).and_then(|mut server| server.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[serve] {error}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RebuildReason {
    Explicit,
    Open,
    List,
    SwitchSource,
    Watch,
}

pub struct PreviewServer {
    args: ServeArgs,
    project: PreviewProject,
    manifest_path: PathBuf,
    crate_dir: PathBuf,
    current_source: Option<PathBuf>,
    current_preview: Option<String>,
    descriptors: Vec<PreviewDescriptorJson>,
    loaded: Option<LoadedPreviewLibrary>,
    backend: Option<Box<dyn PlatformBackend>>,
    host: Option<PreviewHost>,
    retired_host: Option<PreviewHost>,
    build_index: u64,
    last_seen_mtime: SystemTime,
    build_in_progress: bool,
    has_built: bool,
    active_file: Option<PathBuf>,
    shutting_down: bool,
    initialized: bool,
}

impl PreviewServer {
    pub fn new(args: ServeArgs) -> Result<Self, String> {
        let project = prepare_project(&args)?;
        let manifest_path = project.manifest_path.clone();
        let crate_dir = project.crate_dir.clone();
        let current_source = args.source.clone();
        let current_preview = args.preview.clone();

        Ok(Self {
            args,
            project,
            manifest_path,
            crate_dir,
            current_source,
            current_preview,
            descriptors: Vec::new(),
            loaded: None,
            backend: None,
            host: None,
            retired_host: None,
            build_index: 0,
            last_seen_mtime: SystemTime::UNIX_EPOCH,
            build_in_progress: false,
            has_built: false,
            active_file: None,
            shutting_down: false,
            initialized: false,
        })
    }

    pub fn run(&mut self) -> Result<(), String> {
        let (tx, rx) = mpsc::sync_channel::<String>(64);
        thread::Builder::new()
            .name("scarlet-ui-preview-stdin".into())
            .spawn(move || {
                let stdin = io::stdin();
                let reader = io::BufReader::new(stdin.lock());
                for line in reader.lines() {
                    match line {
                        Ok(text) => {
                            if tx.send(text).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("failed to spawn stdin reader: {error}"))?;

        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        self.last_seen_mtime = self.snapshot_mtime()?;

        while !self.shutting_down {
            loop {
                match rx.try_recv() {
                    Ok(line) => {
                        self.handle_line(&line, &mut stdout)?;
                        if self.shutting_down {
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        handle_disconnect(self);
                        break;
                    }
                }
            }
            if self.shutting_down {
                break;
            }

            if self.host.is_none() && !self.build_in_progress {
                let timeout = Duration::from_millis(self.args.poll_interval().as_millis() as u64);
                match rx.recv_timeout(timeout) {
                    Ok(line) => self.handle_line(&line, &mut stdout)?,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            if self.initialized && !self.build_in_progress && self.has_built {
                if let Ok(current) = self.snapshot_mtime()
                    && current > self.last_seen_mtime
                {
                    self.last_seen_mtime = current;
                    if self.host.is_some() {
                        if let Err(error) =
                            self.rebuild_and_apply(&mut stdout, RebuildReason::Watch)
                        {
                            eprintln!("[serve] watch reload failed: {}", error.message);
                        }
                    } else {
                        match self.rebuild(&mut stdout, RebuildReason::Watch) {
                            Ok(library) => self.loaded = Some(library),
                            Err(error) => {
                                eprintln!("[serve] watch rebuild failed: {}", error.message)
                            }
                        }
                    }
                }
            }

            let mut window_closed = false;
            if let Some(host) = self.host.as_mut() {
                match host.tick(Duration::from_millis(16)) {
                    Ok(true) => {}
                    Ok(false) => {
                        host.close();
                        window_closed = true;
                    }
                    Err(error) => {
                        eprintln!("[serve] host tick failed: {error}");
                        host.close();
                        window_closed = true;
                    }
                }
            }
            if window_closed {
                if let Some(mut host) = self.host.take() {
                    host.close();
                    self.retired_host = Some(host);
                }
                self.emit_notification(&mut stdout, "preview/windowClosed", json!({}))?;
            }
        }

        Ok(())
    }

    fn handle_line(&mut self, line: &str, stdout: &mut impl Write) -> Result<(), String> {
        match rpc::parse_message(line) {
            Ok(Incoming::Notification(notification)) => {
                self.handle_notification(notification);
                Ok(())
            }
            Ok(Incoming::Request(request)) => {
                let id = request.id.clone();
                let response = match self.handle_request(request, stdout) {
                    Ok(result) => Response::success(id, result),
                    Err(error) => Response::error(id, error),
                };
                self.emit_response(stdout, response)?;
                Ok(())
            }
            Err(error) => {
                eprintln!("[serve] invalid JSON-RPC message: {}", error.message);
                let response = Response::error(rpc::RequestId::Null, error);
                self.emit_response(stdout, response)?;
                Ok(())
            }
        }
    }

    fn handle_request(
        &mut self,
        request: Request,
        stdout: &mut impl Write,
    ) -> Result<Value, ErrorObject> {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(),
            "preview/list" => self.handle_preview_list(request.params, stdout),
            "preview/open" => self.handle_preview_open(request.params, stdout),
            "preview/switchSource" => self.handle_switch_source(request.params, stdout),
            "preview/switchPreview" => self.handle_switch_preview(request.params, stdout),
            "preview/rebuild" => self.handle_preview_rebuild(stdout),
            "preview/closeWindow" => self.handle_close_window(stdout),
            "shutdown" => self.handle_shutdown(),
            _ => Err(method_dispatch_error(request.method.as_str())),
        }
    }

    fn handle_notification(&mut self, notification: Notification) {
        match notification.method.as_str() {
            "workspace/didChangeActiveFile" => {
                match get_required_str_param(&notification.params, "path") {
                    Ok(path) => {
                        eprintln!("[serve] active file: {path}");
                        apply_active_file(self, PathBuf::from(path));
                    }
                    Err(error) => {
                        eprintln!("[serve] invalid didChangeActiveFile: {}", error.message)
                    }
                }
            }
            method => eprintln!("[serve] ignored notification: {method}"),
        }
    }

    fn handle_initialize(&mut self) -> Result<Value, ErrorObject> {
        self.initialized = true;
        Ok(json!({
            "protocolVersion": 1,
            "package": self.project.package_name,
            "capabilities": {
                "switchSource": true,
                "switchPreview": true,
                "watch": true
            }
        }))
    }

    fn handle_preview_list(
        &mut self,
        params: Option<Params>,
        stdout: &mut impl Write,
    ) -> Result<Value, ErrorObject> {
        if let Some(source) = get_str_param(&params, "source")? {
            self.update_source(PathBuf::from(source))?;
        } else {
            let _ = get_object_params(&params)?;
        }

        if self.descriptors.is_empty() {
            let library = self.rebuild(stdout, RebuildReason::List)?;
            if self.host.is_some() {
                self.reload_host(stdout, library, false)?;
            } else {
                self.loaded = Some(library);
            }
        }

        Ok(json!({ "previews": self.descriptors.clone() }))
    }

    fn handle_preview_open(
        &mut self,
        params: Option<Params>,
        stdout: &mut impl Write,
    ) -> Result<Value, ErrorObject> {
        if let Some(source) = get_str_param(&params, "source")? {
            self.update_source(PathBuf::from(source))?;
        }
        let requested_preview = if get_object_params(&params)?.contains_key("preview") {
            get_str_param(&params, "preview")?
        } else {
            self.current_preview.clone()
        };
        let library = self.rebuild(stdout, RebuildReason::Open)?;
        let active = select_preview_json(&self.descriptors, requested_preview.as_deref())?;
        self.current_preview = requested_preview;

        if let Some(host) = self.host.as_mut() {
            host.reload(library)
                .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
            self.loaded = None;
            if let Some(preview) = self.current_preview.as_deref() {
                host.switch_preview(preview)
                    .map_err(|_| ErrorObject::preview_not_found(preview))?;
            }
            self.emit_notification(
                stdout,
                "preview/reloadFinished",
                json!({ "preview": active }),
            )
            .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
        } else {
            self.retired_host = None;
            let backend = self
                .backend
                .get_or_insert_with(|| Box::new(WinitBackend::new()));
            #[allow(unused_mut)]
            let mut host = PreviewHost::new_with_backend(
                library,
                self.current_preview.as_deref(),
                backend.as_mut(),
                self.args.force_scale_milli(),
            )
            .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
            #[cfg(feature = "gpu")]
            if self.args.use_gpu() {
                crate::gpu::setup_gpu_present(&mut host);
            }
            self.host = Some(host);
            self.loaded = None;
        }

        Ok(json!({ "preview": active }))
    }

    fn handle_switch_source(
        &mut self,
        params: Option<Params>,
        stdout: &mut impl Write,
    ) -> Result<Value, ErrorObject> {
        let source = get_required_str_param(&params, "source")?;
        let requested_preview = get_str_param(&params, "preview")?;
        let has_preview_key = get_object_params(&params)?.contains_key("preview");
        let staged_preview = if has_preview_key {
            requested_preview.clone()
        } else {
            self.current_preview.clone()
        };
        self.update_source(PathBuf::from(source))?;

        let library = self.rebuild(stdout, RebuildReason::SwitchSource)?;
        let active = select_preview_json(&self.descriptors, staged_preview.as_deref())?;
        self.current_preview = staged_preview;

        if let Some(host) = self.host.as_mut() {
            host.reload(library)
                .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
            self.loaded = None;
            if let Some(preview) = self.current_preview.as_deref() {
                host.switch_preview(preview)
                    .map_err(|_| ErrorObject::preview_not_found(preview))?;
            }
            self.emit_notification(
                stdout,
                "preview/reloadFinished",
                json!({ "preview": active }),
            )
            .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
        } else {
            self.loaded = Some(library);
        }

        Ok(json!({ "preview": active }))
    }

    fn handle_switch_preview(
        &mut self,
        params: Option<Params>,
        stdout: &mut impl Write,
    ) -> Result<Value, ErrorObject> {
        let preview = get_required_str_param(&params, "preview")?;
        let active = find_descriptor(&self.descriptors, &preview)
            .cloned()
            .ok_or_else(|| ErrorObject::preview_not_found(&preview))?;
        let host = self
            .host
            .as_mut()
            .ok_or_else(|| ErrorObject::preview_host_window_error("no open preview window"))?;
        host.switch_preview(&preview)
            .map_err(|_| ErrorObject::preview_not_found(&preview))?;
        self.current_preview = Some(preview.clone());
        self.emit_notification(
            stdout,
            "preview/reloadFinished",
            json!({ "preview": active }),
        )
        .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
        Ok(json!({ "preview": active }))
    }

    fn handle_preview_rebuild(&mut self, stdout: &mut impl Write) -> Result<Value, ErrorObject> {
        let previews = self.rebuild_and_apply(stdout, RebuildReason::Explicit)?;
        Ok(json!({ "previews": previews }))
    }

    fn handle_close_window(&mut self, stdout: &mut impl Write) -> Result<Value, ErrorObject> {
        if let Some(mut host) = self.host.take() {
            host.close();
            self.retired_host = Some(host);
            self.emit_notification(stdout, "preview/windowClosed", json!({}))
                .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
        }
        Ok(json!({}))
    }

    fn handle_shutdown(&mut self) -> Result<Value, ErrorObject> {
        mark_shutdown(self);
        Ok(json!({}))
    }

    fn update_source(&mut self, source: PathBuf) -> Result<(), ErrorObject> {
        if self.current_source.as_ref() == Some(&source) {
            return Ok(());
        }
        let mut staged_args = self.args.clone();
        staged_args.source = Some(source.clone());
        let project = prepare_project(&staged_args)
            .map_err(|error| ErrorObject::invalid_params(Some(&error)))?;
        self.args = staged_args;
        self.manifest_path = project.manifest_path.clone();
        self.crate_dir = project.crate_dir.clone();
        self.project = project;
        self.current_source = Some(source);
        self.loaded = None;
        self.descriptors.clear();
        Ok(())
    }

    fn rebuild_and_apply(
        &mut self,
        stdout: &mut impl Write,
        reason: RebuildReason,
    ) -> Result<Vec<PreviewDescriptorJson>, ErrorObject> {
        let library = self.rebuild(stdout, reason)?;
        let previews = self.descriptors.clone();
        if self.host.is_some() {
            self.reload_host(stdout, library, true)?;
        } else {
            self.loaded = Some(library);
        }
        Ok(previews)
    }

    fn reload_host(
        &mut self,
        stdout: &mut impl Write,
        library: LoadedPreviewLibrary,
        emit_finished: bool,
    ) -> Result<(), ErrorObject> {
        if let Some(host) = self.host.as_mut() {
            host.reload(library)
                .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
            self.loaded = None;
            if let Some(preview) = self.current_preview.as_deref() {
                host.switch_preview(preview)
                    .map_err(|_| ErrorObject::preview_not_found(preview))?;
            }
            if emit_finished {
                self.emit_notification(stdout, "preview/reloadFinished", json!({}))
                    .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
            }
        }
        Ok(())
    }

    fn rebuild(
        &mut self,
        stdout: &mut impl Write,
        reason: RebuildReason,
    ) -> Result<LoadedPreviewLibrary, ErrorObject> {
        self.build_in_progress = true;
        eprintln!("[serve] rebuild started: {reason:?}");
        if let Err(error) = self.emit_notification(
            stdout,
            "preview/buildStarted",
            json!({ "source": self.current_source.as_ref().map(|path| path.display().to_string()) }),
        ) {
            self.build_in_progress = false;
            return Err(ErrorObject::preview_host_window_error(&error));
        }

        let result = build_and_load(
            &self.args,
            &self.project,
            self.build_index,
            CargoStdout::Null,
        );
        match result {
            Ok(library) => {
                self.descriptors = preview_jsons(&library);
                self.build_in_progress = false;
                self.has_built = true;
                self.build_index = self.build_index.wrapping_add(1);
                self.last_seen_mtime = self.snapshot_mtime().unwrap_or(self.last_seen_mtime);
                self.emit_notification(
                    stdout,
                    "preview/buildFinished",
                    json!({ "previews": self.descriptors.clone() }),
                )
                .map_err(|error| ErrorObject::preview_host_window_error(&error))?;
                Ok(library)
            }
            Err(error) => {
                self.build_in_progress = false;
                self.last_seen_mtime = self.snapshot_mtime().unwrap_or(self.last_seen_mtime);
                let _ = self.emit_notification(
                    stdout,
                    "preview/buildFailed",
                    json!({ "message": error }),
                );
                Err(build_or_load_error(&error))
            }
        }
    }

    fn snapshot_mtime(&self) -> Result<SystemTime, String> {
        latest_source_mtime(&self.crate_dir)
    }

    fn emit_notification(
        &mut self,
        stdout: &mut impl Write,
        method: &str,
        params: Value,
    ) -> Result<(), String> {
        let notification = Notification {
            jsonrpc: String::from("2.0"),
            method: method.to_string(),
            params: Some(Params::Object(value_to_params(params)?)),
        };
        self.write_protocol(stdout, &notification)
    }

    fn emit_response(&mut self, stdout: &mut impl Write, response: Response) -> Result<(), String> {
        self.write_protocol(stdout, &response)
    }

    fn write_protocol<T: serde::Serialize>(
        &mut self,
        stdout: &mut impl Write,
        message: &T,
    ) -> Result<(), String> {
        let line = rpc::serialize_message(message).map_err(|error| error.message)?;
        writeln!(stdout, "{line}").map_err(|error| format!("failed to write stdout: {error}"))?;
        stdout
            .flush()
            .map_err(|error| format!("failed to flush stdout: {error}"))
    }
}

fn value_to_params(value: Value) -> Result<serde_json::Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(String::from("notification params must be an object")),
    }
}

fn preview_jsons(library: &LoadedPreviewLibrary) -> Vec<PreviewDescriptorJson> {
    library
        .previews()
        .iter()
        .map(PreviewDescriptorJson::from_descriptor)
        .collect()
}

fn select_preview_json(
    previews: &[PreviewDescriptorJson],
    preview: Option<&str>,
) -> Result<PreviewDescriptorJson, ErrorObject> {
    if let Some(preview) = preview {
        find_descriptor(previews, preview)
            .cloned()
            .ok_or_else(|| ErrorObject::preview_not_found(preview))
    } else {
        previews
            .first()
            .cloned()
            .ok_or_else(|| ErrorObject::preview_not_found("<none>"))
    }
}

fn find_descriptor<'a>(
    descriptors: &'a [PreviewDescriptorJson],
    key: &str,
) -> Option<&'a PreviewDescriptorJson> {
    descriptors
        .iter()
        .find(|descriptor| descriptor.id == key || descriptor.name == key)
}

fn method_dispatch_error(method: &str) -> ErrorObject {
    ErrorObject::method_not_found().with_data(json!({ "method": method }))
}

fn build_or_load_error(message: &str) -> ErrorObject {
    if message.contains("scarlet_ui_preview_entry")
        || message.contains("Symbol not found")
        || message.contains("image not found")
        || message.contains("dlopen")
    {
        ErrorObject::preview_library_load_failed(message)
    } else {
        ErrorObject::build_failed(message)
    }
}

fn get_required_str_param(params: &Option<Params>, key: &str) -> Result<String, ErrorObject> {
    get_str_param(params, key)?.ok_or_else(|| {
        ErrorObject::invalid_params(Some(&format!("missing required string param: {key}")))
    })
}

fn get_str_param(params: &Option<Params>, key: &str) -> Result<Option<String>, ErrorObject> {
    let object = get_object_params(params)?;
    match object.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(ErrorObject::invalid_params(Some(&format!(
            "param {key} must be a string"
        )))),
    }
}

fn get_object_params(
    params: &Option<Params>,
) -> Result<&serde_json::Map<String, Value>, ErrorObject> {
    match params {
        Some(Params::Object(map)) => Ok(map),
        Some(Params::Array(_)) => Err(ErrorObject::invalid_params(Some(
            "params must be an object",
        ))),
        None => Ok(&EMPTY_PARAMS),
    }
}

fn apply_active_file(state: &mut PreviewServer, path: PathBuf) {
    state.active_file = Some(path);
}

fn mark_shutdown(state: &mut PreviewServer) {
    state.shutting_down = true;
}

fn handle_disconnect(state: &mut PreviewServer) {
    state.shutting_down = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object_params(value: Value) -> Option<Params> {
        Some(Params::Object(value.as_object().expect("object").clone()))
    }

    #[test]
    fn get_str_param_returns_some_none_and_err() {
        let params = object_params(json!({ "source": "src/lib.rs", "count": 1 }));

        assert_eq!(
            get_str_param(&params, "source").expect("source param"),
            Some(String::from("src/lib.rs"))
        );
        assert_eq!(get_str_param(&params, "missing").expect("missing"), None);
        assert_eq!(
            get_str_param(&params, "count")
                .expect_err("wrong type")
                .code,
            rpc::INVALID_PARAMS
        );
    }

    #[test]
    fn get_object_params_rejects_array_params() {
        let params = Some(Params::Array(vec![json!("x")]));

        assert_eq!(
            get_object_params(&params).expect_err("array params").code,
            rpc::INVALID_PARAMS
        );
    }

    #[test]
    fn unknown_method_dispatch_returns_method_not_found() {
        let error = method_dispatch_error("missing/method");

        assert_eq!(error.code, rpc::METHOD_NOT_FOUND);
    }

    #[test]
    fn missing_required_param_returns_invalid_params() {
        let params = object_params(json!({}));

        assert_eq!(
            get_required_str_param(&params, "preview")
                .expect_err("missing preview")
                .code,
            rpc::INVALID_PARAMS
        );
    }

    #[test]
    fn active_file_update_sets_path_without_rebuild_flags() {
        let mut server = PreviewServer::for_tests();
        apply_active_file(&mut server, PathBuf::from("src/main.rs"));

        assert_eq!(server.active_file, Some(PathBuf::from("src/main.rs")));
        assert!(!server.build_in_progress);
    }

    #[test]
    fn mark_shutdown_sets_shutdown_flag() {
        let mut server = PreviewServer::for_tests();
        mark_shutdown(&mut server);

        assert!(server.shutting_down);
    }

    #[test]
    fn handle_disconnect_sets_shutdown_flag() {
        let mut server = PreviewServer::for_tests();
        handle_disconnect(&mut server);

        assert!(server.shutting_down);
    }

    #[test]
    fn find_descriptor_matches_id_or_name() {
        let descriptors = vec![PreviewDescriptorJson {
            id: String::from("counter_preview"),
            name: String::from("Counter Preview"),
        }];

        assert_eq!(
            find_descriptor(&descriptors, "counter_preview")
                .map(|descriptor| descriptor.name.as_str()),
            Some("Counter Preview")
        );
        assert_eq!(
            find_descriptor(&descriptors, "Counter Preview")
                .map(|descriptor| descriptor.id.as_str()),
            Some("counter_preview")
        );
        assert!(find_descriptor(&descriptors, "Missing Preview").is_none());
    }

    #[test]
    fn parse_error_response_uses_null_id_and_parse_error_code() {
        let response = Response::error(rpc::RequestId::Null, ErrorObject::parse_error());
        let value: Value =
            serde_json::from_str(&rpc::serialize_message(&response).unwrap()).unwrap();

        assert_eq!(value["id"], Value::Null);
        assert_eq!(value["error"]["code"], rpc::PARSE_ERROR);
    }

    #[test]
    fn failed_source_update_leaves_args_unchanged() {
        let mut server = PreviewServer::for_tests();
        let original_args = server.args.clone();
        let original_manifest_path = server.manifest_path.clone();
        let original_crate_dir = server.crate_dir.clone();
        let original_project_manifest_path = server.project.manifest_path.clone();
        let original_project_crate_dir = server.project.crate_dir.clone();
        let original_project_package_name = server.project.package_name.clone();
        let original_source = server.current_source.clone();
        server.descriptors = vec![PreviewDescriptorJson {
            id: String::from("counter_preview"),
            name: String::from("Counter Preview"),
        }];

        let result = server.update_source(PathBuf::from("/definitely/not/a/source.rs"));

        assert!(result.is_err());
        assert_eq!(server.args.source, original_args.source);
        assert_eq!(server.manifest_path, original_manifest_path);
        assert_eq!(server.crate_dir, original_crate_dir);
        assert_eq!(server.project.manifest_path, original_project_manifest_path);
        assert_eq!(server.project.crate_dir, original_project_crate_dir);
        assert_eq!(server.project.package_name, original_project_package_name);
        assert_eq!(server.current_source, original_source);
        assert!(!server.descriptors.is_empty());
    }

    #[test]
    fn rebuild_reason_is_stateless() {
        assert_eq!(RebuildReason::Watch, RebuildReason::Watch);
        assert_ne!(RebuildReason::Watch, RebuildReason::Explicit);
    }

    impl PreviewServer {
        fn for_tests() -> Self {
            let manifest_path = PathBuf::from("/tmp/scarlet-ui-preview-test/Cargo.toml");
            let crate_dir = PathBuf::from("/tmp/scarlet-ui-preview-test");
            let args = ServeArgs {
                manifest_path: manifest_path.clone(),
                source: None,
                target: None,
                features: None,
                preview: None,
                poll_ms: 250,
                force_scale: None,
                #[cfg(feature = "gpu")]
                gpu: false,
                #[cfg(feature = "gpu")]
                no_gpu: false,
            };
            let project = PreviewProject {
                manifest_path: manifest_path.clone(),
                crate_dir: crate_dir.clone(),
                package_name: String::from("test-package"),
                target: crate::build::PreviewTarget::Library,
            };

            Self {
                args,
                project,
                manifest_path,
                crate_dir,
                current_source: None,
                current_preview: None,
                descriptors: Vec::new(),
                loaded: None,
                backend: None,
                host: None,
                retired_host: None,
                build_index: 0,
                last_seen_mtime: SystemTime::UNIX_EPOCH,
                build_in_progress: false,
                has_built: false,
                active_file: None,
                shutting_down: false,
                initialized: false,
            }
        }
    }
}
