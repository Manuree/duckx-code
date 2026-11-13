//! The language server worker. One tokio task owns every server process, the text
//! each server was last sent, and the conversion between rope bytes and LSP positions.
//! `App` talks to it only through [`LspRequest`] and [`LspEvent`] channels.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ropey::Rope;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::plain::{self, RawRange};
use super::position::{self, Encoding};
use super::protocol::{
    ActionOffer, CompletionItem, CompletionTrigger, Diagnostic, JumpTarget, LspEvent, LspRequest,
    QueryKind, ServerStatus, Severity, TextChange, TextPatch,
};
use super::registry::{self, ServerSpec};
use super::{transport, uri};

/// A server that crashes after it was ready is started again this many times.
const MAX_RESTARTS: u32 = 3;
const SHUTDOWN_REPLY: Duration = Duration::from_millis(1500);
const EXIT_WAIT: Duration = Duration::from_millis(500);
const STDERR_TAIL_WAIT: Duration = Duration::from_millis(300);
/// Progress text reaches the status bar at most this often per server.
const BUSY_INTERVAL: Duration = Duration::from_millis(100);
/// Typing waits this long so a burst of characters becomes one completion request.
const COMPLETION_WAIT: Duration = Duration::from_millis(50);
/// Wait after a change before asking for diagnostics of the unsaved text.
const DIAGNOSTIC_WAIT: Duration = Duration::from_millis(200);
/// Document versions kept so a late diagnostic can be mapped from the text it was computed on.
const VERSIONS: usize = 64;

/// The pipes of one running server.
pub struct Connection {
    pub input: Box<dyn AsyncWrite + Send + Unpin>,
    pub output: Box<dyn AsyncRead + Send + Unpin>,
    pub stderr: Option<Box<dyn AsyncRead + Send + Unpin>>,
    /// Killed when the worker stops. `None` for in-process servers in tests.
    pub child: Option<Child>,
    /// Where stderr is copied.
    pub log: Option<PathBuf>,
}

/// Starts `spec` rooted at the given folder. `ErrorKind::NotFound` means the
/// program is not installed, and the next candidate is tried.
pub type Launch = Arc<dyn Fn(&ServerSpec, &Path) -> io::Result<Connection> + Send + Sync>;

pub struct LspHandle {
    requests: mpsc::UnboundedSender<LspRequest>,
    task: JoinHandle<()>,
}

impl LspHandle {
    pub fn send(&self, request: LspRequest) {
        let _ = self.requests.send(request);
    }

    /// Asks every server to exit, then waits at most `limit` for the worker.
    pub async fn shutdown(self, limit: Duration) {
        let _ = self.requests.send(LspRequest::Shutdown);
        let _ = tokio::time::timeout(limit, self.task).await;
    }
}

/// Starts the worker with real server processes. stderr of each server is
/// copied to `log_dir/lsp-<name>.log`.
pub fn spawn(log_dir: Option<PathBuf>) -> (LspHandle, mpsc::UnboundedReceiver<LspEvent>) {
    spawn_with(Arc::new(move |spec, root| {
        launch_process(spec, root, log_dir.as_deref())
    }))
}

pub fn spawn_with(launch: Launch) -> (LspHandle, mpsc::UnboundedReceiver<LspEvent>) {
    let (requests, request_rx) = mpsc::unbounded_channel();
    let (events, event_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(request_rx, events, launch));
    (LspHandle { requests, task }, event_rx)
}

fn launch_process(
    spec: &ServerSpec,
    root: &Path,
    log_dir: Option<&Path>,
) -> io::Result<Connection> {
    // The program is looked up on PATH (with PATHEXT on Windows) and started
    // directly by its full path, never through a shell.
    let program = super::detect::find_program(&spec.command)
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let mut child = spawn_program(&program, &spec.args, root)?;
    let missing = || io::Error::other("server pipe was not created");
    let input = child.stdin.take().ok_or_else(missing)?;
    let output = child.stdout.take().ok_or_else(missing)?;
    let stderr = child.stderr.take();
    Ok(Connection {
        input: Box::new(input),
        output: Box::new(output),
        stderr: stderr.map(|pipe| Box::new(pipe) as Box<dyn AsyncRead + Send + Unpin>),
        child: Some(child),
        log: log_dir.map(|dir| dir.join(format!("lsp-{}.log", log_name(&spec.name)))),
    })
}

/// A server name made safe for a file name: `my server/1` becomes `my-server-1`.
fn log_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn spawn_program(program: &Path, args: &[String], root: &Path) -> io::Result<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if root.is_dir() {
        command.current_dir(root);
    }
    // Without a console of its own, the server cannot change the editor's console modes.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command.spawn()
}

enum Internal {
    Message {
        server: usize,
        instance: u64,
        body: Vec<u8>,
    },
    Exited {
        server: usize,
        instance: u64,
        reason: String,
    },
    Restart {
        server: usize,
    },
    /// Send the progress update held back by the rate limit.
    Flush {
        server: usize,
    },
    /// The debounce elapsed; send the query if it is still the current one.
    Dispatch {
        token: u64,
    },
    /// Ask for diagnostics of one file, if this is still the latest change.
    Pull {
        path: PathBuf,
        generation: u64,
    },
    /// Ask for semantic tokens of one file, if this is still the latest change.
    Tokens {
        path: PathBuf,
        generation: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Restarting,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncKind {
    None,
    Full,
    Incremental,
}

#[derive(Debug, Clone)]
struct Capabilities {
    encoding: Encoding,
    change: SyncKind,
    /// `Some(include_text)` when the server wants `didSave`.
    save: Option<bool>,
    hover: bool,
    definition: bool,
    completion: bool,
    /// `completionItem/resolve`.
    resolve: bool,
    /// `textDocument/signatureHelp`.
    signature: bool,
    /// `textDocument/diagnostic`, so checks can use the unsaved buffer.
    pull: bool,
    /// `textDocument/semanticTokens/full`.
    semantic: bool,
    rename: bool,
    format: bool,
    references: bool,
    code_action: bool,
    action_resolve: bool,
    token_types: Vec<String>,
    token_modifiers: Vec<String>,
    triggers: Vec<String>,
}

impl Capabilities {
    fn parse(result: &Value) -> Self {
        let caps = &result["capabilities"];
        let encoding = Encoding::from_name(caps["positionEncoding"].as_str());
        let sync = &caps["textDocumentSync"];
        let kind = |value: &Value| match value.as_u64() {
            Some(1) => SyncKind::Full,
            Some(2) => SyncKind::Incremental,
            _ => SyncKind::None,
        };
        let (change, save) = if sync.is_number() {
            (kind(sync), Some(false))
        } else {
            let save = match &sync["save"] {
                Value::Bool(true) => Some(false),
                Value::Object(options) => Some(
                    options
                        .get("includeText")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
                _ => None,
            };
            (kind(&sync["change"]), save)
        };
        let triggers = caps["completionProvider"]["triggerCharacters"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let names = |value: &Value| {
            value
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let legend = &caps["semanticTokensProvider"]["legend"];
        Self {
            encoding,
            change,
            save,
            hover: caps.get("hoverProvider").is_some_and(flag),
            definition: caps.get("definitionProvider").is_some_and(flag),
            completion: caps.get("completionProvider").is_some(),
            resolve: caps["completionProvider"]["resolveProvider"]
                .as_bool()
                .unwrap_or(false),
            signature: caps.get("signatureHelpProvider").is_some_and(flag),
            pull: caps.get("diagnosticProvider").is_some_and(flag),
            semantic: caps.get("semanticTokensProvider").is_some_and(flag),
            rename: caps.get("renameProvider").is_some_and(flag),
            format: caps.get("documentFormattingProvider").is_some_and(flag),
            references: caps.get("referencesProvider").is_some_and(flag),
            code_action: caps.get("codeActionProvider").is_some_and(flag),
            action_resolve: caps["codeActionProvider"]["resolveProvider"]
                .as_bool()
                .unwrap_or(false),
            token_types: names(&legend["tokenTypes"]),
            token_modifiers: names(&legend["tokenModifiers"]),
            triggers,
        }
    }
}

/// `true` for `true` or an options object. Missing means the server does not offer it.
fn flag(value: &Value) -> bool {
    match value {
        Value::Bool(on) => *on,
        Value::Object(_) => true,
        _ => false,
    }
}

struct Server {
    group: String,
    spec: Arc<ServerSpec>,
    root: PathBuf,
    instance: u64,
    writer: Option<mpsc::UnboundedSender<Value>>,
    /// Dropping this kills the process.
    kill: Option<oneshot::Sender<()>>,
    state: State,
    caps: Capabilities,
    next_id: i64,
    init_id: Option<i64>,
    shutdown_id: Option<i64>,
    shutdown_acked: bool,
    restarts: u32,
    /// Work-done progress in flight: token, title, and the label shown for it.
    progress: Vec<Progress>,
    /// Last status sent to `App`, so repeated progress reports do not redraw.
    shown: Option<ServerStatus>,
    shown_at: Option<Instant>,
    /// A progress update waiting for [`BUSY_INTERVAL`] to pass.
    held: Option<ServerStatus>,
}

impl Server {
    fn send(&self, message: Value) {
        if let Some(writer) = &self.writer {
            let _ = writer.send(message);
        }
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }
}

struct Progress {
    token: String,
    title: String,
    label: String,
}

struct Inflight {
    token: u64,
    path: PathBuf,
    at: usize,
    kind: QueryKind,
    server: usize,
    rpc: Option<i64>,
    /// The text the request was about. A later edit must not move its ranges.
    text: Option<Rope>,
    /// Document version of `text`.
    version: u64,
}

struct PullSnap {
    path: PathBuf,
    version: u64,
    text: Rope,
}

struct Doc {
    server: usize,
    uri: String,
    language_id: String,
    version: u64,
    /// What the server has been told the file contains.
    text: Rope,
    /// `didOpen` reached the current server process.
    opened: bool,
    /// `(version, text)` for recent versions, newest last.
    versions: VecDeque<(u64, Rope)>,
    /// Bumped on every change so a delayed diagnostic pull can see it is stale.
    pull_generation: u64,
    /// `textDocument/diagnostic` already sent for this file.
    pull_rpc: Option<i64>,
    token_generation: u64,
    token_rpc: Option<i64>,
}

impl Doc {
    /// `None` when the server named a version this process no longer has.
    fn text_for(&self, version: Option<u64>) -> Option<(u64, Rope)> {
        match version {
            Some(version) => self
                .versions
                .iter()
                .rev()
                .find(|(stored, _)| *stored == version)
                .map(|(stored, text)| (*stored, text.clone())),
            None => Some((self.version, self.text.clone())),
        }
    }
}

fn remember(doc: &mut Doc) {
    doc.versions.push_back((doc.version, doc.text.clone()));
    if doc.versions.len() > VERSIONS {
        doc.versions.pop_front();
    }
}

struct Manager {
    events: mpsc::UnboundedSender<LspEvent>,
    internal: mpsc::UnboundedSender<Internal>,
    launch: Launch,
    servers: Vec<Server>,
    by_root: HashMap<(String, PathBuf), usize>,
    docs: HashMap<PathBuf, Doc>,
    /// Queries waiting to be sent, or already sent and waiting for a reply.
    inflight: Vec<Inflight>,
    /// `textDocument/diagnostic` requests, keyed by JSON-RPC id.
    pulls: HashMap<i64, PullSnap>,
    /// `textDocument/semanticTokens/full` requests, keyed by JSON-RPC id.
    tokens: HashMap<i64, PullSnap>,
    /// Server groups with no installed program. Not retried this session.
    missing: HashSet<String>,
    next_instance: u64,
    shutting_down: bool,
    prefs: crate::settings::LspPrefs,
}

async fn run(
    mut requests: mpsc::UnboundedReceiver<LspRequest>,
    events: mpsc::UnboundedSender<LspEvent>,
    launch: Launch,
) {
    let (internal, mut internal_rx) = mpsc::unbounded_channel();
    let mut manager = Manager {
        events,
        internal,
        launch,
        servers: Vec::new(),
        by_root: HashMap::new(),
        docs: HashMap::new(),
        inflight: Vec::new(),
        pulls: HashMap::new(),
        tokens: HashMap::new(),
        missing: HashSet::new(),
        next_instance: 0,
        shutting_down: false,
        prefs: crate::settings::LspPrefs::default(),
    };
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                None | Some(LspRequest::Shutdown) => break,
                Some(LspRequest::Reset) => {
                    manager.shutdown(&mut internal_rx).await;
                    manager.clear_after_reset();
                    manager.emit(LspEvent::ResetDone);
                }
                Some(request) => manager.handle_request(request),
            },
            Some(message) = internal_rx.recv() => manager.handle_internal(message),
        }
    }
    manager.shutdown(&mut internal_rx).await;
}

impl Manager {
    fn emit(&self, event: LspEvent) {
        let _ = self.events.send(event);
    }

    fn status(&mut self, index: usize, status: ServerStatus) {
        let server = &mut self.servers[index];
        server.shown = Some(status.clone());
        server.shown_at = Some(Instant::now());
        server.held = None;
        let server = &self.servers[index];
        self.emit(LspEvent::Status {
            group: server.group.clone(),
            name: server.spec.name.clone(),
            status,
        });
    }

    fn handle_request(&mut self, request: LspRequest) {
        match request {
            LspRequest::Open {
                path,
                servers,
                text,
                version,
                workspace,
            } => {
                let Some(first) = servers.first().cloned() else {
                    return;
                };
                self.close(&path);
                if self.missing.contains(&first.name) {
                    return;
                }
                let root = registry::find_root(&path, &workspace, &first.root_markers);
                let server = match self.by_root.get(&(first.name.clone(), root.clone())) {
                    Some(&index) => index,
                    None => match self.start(&servers, root) {
                        Some(index) => index,
                        None => return,
                    },
                };
                let language_id = self.servers[server]
                    .spec
                    .language_id(&path)
                    .unwrap_or("plaintext")
                    .to_string();
                let mut doc = Doc {
                    server,
                    uri: uri::from_path(&path),
                    language_id,
                    version,
                    text,
                    opened: false,
                    versions: VecDeque::new(),
                    pull_generation: 0,
                    pull_rpc: None,
                    token_generation: 0,
                    token_rpc: None,
                };
                remember(&mut doc);
                if self.servers[server].state == State::Ready {
                    send_open(&self.servers[server], &mut doc);
                }
                let opened = doc.opened;
                self.docs.insert(path.clone(), doc);
                if opened {
                    self.schedule_pull(&path);
                    self.schedule_tokens(&path);
                }
            }
            LspRequest::Change {
                path,
                text,
                version,
                change,
            } => self.change(&path, text, version, change),
            LspRequest::Save { path } => {
                let Some(doc) = self.docs.get(&path) else {
                    return;
                };
                let server = &self.servers[doc.server];
                if !doc.opened || server.state != State::Ready {
                    return;
                }
                if let Some(include_text) = server.caps.save {
                    let mut params = json!({"textDocument": {"uri": doc.uri}});
                    if include_text {
                        params["text"] = Value::String(doc.text.to_string());
                    }
                    server.notify("textDocument/didSave", params);
                }
            }
            LspRequest::Close { path } => {
                self.drop_queries(&path);
                self.close(&path);
            }
            LspRequest::Query {
                token,
                path,
                at,
                kind,
            } => self.query(token, path, at, kind),
            LspRequest::Cancel { token } => self.cancel(token),
            LspRequest::Shutdown | LspRequest::Reset => {}
            LspRequest::Configure(prefs) => self.configure(prefs),
            LspRequest::Stop { name } => self.stop_named(&name),
            LspRequest::Restart { name } => {
                self.stop_named(&name);
                // Detect again: a program installed since the last try is used now.
                self.missing.clear();
            }
        }
    }

    fn configure(&mut self, prefs: crate::settings::LspPrefs) {
        self.prefs = prefs;
        for server in &self.servers {
            if server.state == State::Ready {
                server.notify("workspace/didChangeConfiguration", json!({"settings": {}}));
            }
        }
    }

    fn query(&mut self, token: u64, path: PathBuf, at: usize, kind: QueryKind) {
        let Some(doc) = self.docs.get(&path) else {
            self.empty_query(token, &kind);
            return;
        };
        let server = doc.server;
        let version = doc.version;
        self.supersede(&path, &kind);
        self.inflight.push(Inflight {
            token,
            path,
            at,
            kind: kind.clone(),
            server,
            rpc: None,
            text: None,
            version,
        });
        let wait = match &kind {
            QueryKind::Completion {
                immediate: false, ..
            } => COMPLETION_WAIT,
            _ => Duration::ZERO,
        };
        if wait.is_zero() {
            self.dispatch(token);
            return;
        }
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = internal.send(Internal::Dispatch { token });
        });
    }

    /// A newer query for the same file and feature replaces the one still in flight.
    fn supersede(&mut self, path: &Path, kind: &QueryKind) {
        let stale: Vec<u64> = self
            .inflight
            .iter()
            .filter(|item| item.path == path && same_feature(&item.kind, kind))
            .map(|item| item.token)
            .collect();
        for token in stale {
            self.cancel(token);
        }
    }

    fn cancel(&mut self, token: u64) {
        let Some(index) = self.inflight.iter().position(|item| item.token == token) else {
            return;
        };
        let item = self.inflight.remove(index);
        if let Some(rpc) = item.rpc {
            let server = &self.servers[item.server];
            server.notify("$/cancelRequest", json!({"id": rpc}));
        }
    }

    fn drop_queries(&mut self, path: &Path) {
        let stale: Vec<u64> = self
            .inflight
            .iter()
            .filter(|item| item.path == path)
            .map(|item| item.token)
            .collect();
        for token in stale {
            self.cancel(token);
        }
    }

    fn dispatch(&mut self, token: u64) {
        let Some(index) = self.inflight.iter().position(|item| item.token == token) else {
            return;
        };
        if self.inflight[index].rpc.is_some() {
            return;
        }
        let path = self.inflight[index].path.clone();
        let at = self.inflight[index].at;
        let kind = self.inflight[index].kind.clone();
        let server_index = self.inflight[index].server;
        let prepared = if let Some(doc) = self.docs.get(&path) {
            let server = &self.servers[server_index];
            let supported = match &kind {
                QueryKind::Hover => server.caps.hover,
                QueryKind::Definition => server.caps.definition,
                QueryKind::Completion { .. } => server.caps.completion,
                QueryKind::Signature => server.caps.signature,
                QueryKind::Resolve { .. } => server.caps.resolve,
                QueryKind::Rename { .. } => server.caps.rename,
                QueryKind::Format => server.caps.format,
                QueryKind::References => server.caps.references,
                QueryKind::CodeAction => server.caps.code_action,
                QueryKind::CodeActionResolve { .. } => server.caps.action_resolve,
            };
            if !doc.opened || server.state != State::Ready {
                None
            } else if !supported {
                let feature = match &kind {
                    QueryKind::Hover => "hover",
                    QueryKind::Definition => "go to definition",
                    QueryKind::Completion { .. } => "completion",
                    QueryKind::Signature => "signature help",
                    QueryKind::Resolve { .. } => "completion resolve",
                    QueryKind::Rename { .. } => "rename",
                    QueryKind::Format => "format",
                    QueryKind::References => "find references",
                    QueryKind::CodeAction => "code action",
                    QueryKind::CodeActionResolve { .. } => "code action",
                };
                Some(Err((server.spec.name.clone(), feature)))
            } else {
                let encoding = server.caps.encoding;
                let (line, character) = position::position_of(&doc.text, at, encoding);
                let uri = doc.uri.clone();
                let method = match &kind {
                    QueryKind::Hover => "textDocument/hover",
                    QueryKind::Definition => "textDocument/definition",
                    QueryKind::Completion { .. } => "textDocument/completion",
                    QueryKind::Signature => "textDocument/signatureHelp",
                    QueryKind::Resolve { .. } => "completionItem/resolve",
                    QueryKind::Rename { .. } => "textDocument/rename",
                    QueryKind::Format => "textDocument/formatting",
                    QueryKind::References => "textDocument/references",
                    QueryKind::CodeAction => "textDocument/codeAction",
                    QueryKind::CodeActionResolve { .. } => "codeAction/resolve",
                };
                let mut params = if matches!(kind, QueryKind::Format) {
                    json!({
                        "textDocument": {"uri": uri},
                        "options": {"tabSize": 4, "insertSpaces": true},
                    })
                } else {
                    json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line, "character": character},
                    })
                };
                if let QueryKind::Completion { trigger, .. } = &kind {
                    params["context"] = match trigger {
                        CompletionTrigger::Invoked => json!({"triggerKind": 1}),
                        CompletionTrigger::Character(character) => {
                            json!({"triggerKind": 2, "triggerCharacter": character})
                        }
                    };
                }
                if let QueryKind::Resolve { item } | QueryKind::CodeActionResolve { action: item } =
                    &kind
                {
                    params = serde_json::from_str(item).unwrap_or(Value::Null);
                }
                if let QueryKind::Rename { new_name } = &kind {
                    params["newName"] = Value::String(new_name.clone());
                }
                if matches!(kind, QueryKind::References) {
                    params["context"] = json!({"includeDeclaration": true});
                }
                if matches!(kind, QueryKind::CodeAction) {
                    params["range"] = json!({
                        "start": {"line": line, "character": character},
                        "end": {"line": line, "character": character},
                    });
                    params["context"] = json!({"diagnostics": []});
                    params
                        .as_object_mut()
                        .map(|object| object.remove("position"));
                }
                Some(Ok((method, params, doc.text.clone())))
            }
        } else {
            None
        };
        let (method, params, text) = match prepared {
            None => {
                self.inflight.retain(|item| item.token != token);
                self.empty_query(token, &kind);
                return;
            }
            Some(Err((name, feature))) => {
                self.emit(LspEvent::Notice(format!(
                    "{name} does not support {feature}"
                )));
                self.inflight.retain(|item| item.token != token);
                self.empty_query(token, &kind);
                return;
            }
            Some(Ok(ready)) => ready,
        };
        let rpc = self.servers[server_index].request(method, params);
        if let Some(item) = self.inflight.iter_mut().find(|item| item.token == token) {
            item.rpc = Some(rpc);
            item.text = Some(text);
        }
    }

    fn jump_targets(&self, server: usize, result: &Value) -> Vec<JumpTarget> {
        let encoding = self.servers[server].caps.encoding;
        plain::definition_targets(result)
            .into_iter()
            .filter_map(|target| {
                let path = uri::to_path(&target.uri)?;
                Some(JumpTarget {
                    path,
                    line: target.range.line,
                    character: target.range.character,
                    end_line: target.range.end_line,
                    end_character: target.range.end_character,
                    encoding,
                })
            })
            .collect()
    }

    fn empty_query(&self, token: u64, kind: &QueryKind) {
        self.emit(match kind {
            QueryKind::Hover => LspEvent::Hover { token, text: None },
            QueryKind::Definition => LspEvent::Definition {
                token,
                locations: Vec::new(),
            },
            QueryKind::References => LspEvent::References {
                token,
                locations: Vec::new(),
            },
            QueryKind::Completion { .. } => LspEvent::Completion {
                token,
                incomplete: false,
                items: Vec::new(),
            },
            QueryKind::Signature => LspEvent::Signature { token, text: None },
            QueryKind::Resolve { .. } => LspEvent::Resolved {
                token,
                detail: String::new(),
                docs: String::new(),
            },
            QueryKind::Rename { .. } | QueryKind::Format | QueryKind::CodeActionResolve { .. } => {
                LspEvent::WorkspaceEdit {
                    token,
                    encoding: Encoding::Utf16,
                    edits: Vec::new(),
                }
            }
            QueryKind::CodeAction => LspEvent::Actions {
                token,
                encoding: Encoding::Utf16,
                actions: Vec::new(),
            },
        });
    }

    fn finish_query(&mut self, server: usize, token: u64, result: &Value) {
        let Some(index) = self.inflight.iter().position(|item| item.token == token) else {
            return;
        };
        let item = self.inflight.remove(index);
        let encoding = self.servers[server].caps.encoding;
        let text = item.text.clone().unwrap_or_else(|| {
            self.docs
                .get(&item.path)
                .map(|doc| doc.text.clone())
                .unwrap_or_default()
        });
        let bytes = |range: &RawRange| {
            (
                position::byte_of(&text, range.line, range.character, encoding),
                position::byte_of(&text, range.end_line, range.end_character, encoding),
            )
        };
        match item.kind {
            QueryKind::Hover => self.emit(LspEvent::Hover {
                token,
                text: plain::hover_text(result),
            }),
            QueryKind::Definition => {
                let locations = self.jump_targets(server, result);
                self.emit(LspEvent::Definition { token, locations });
            }
            QueryKind::References => {
                let locations = self.jump_targets(server, result);
                self.emit(LspEvent::References { token, locations });
            }
            QueryKind::Completion { .. } => {
                let (incomplete, raw) = plain::completion_items(result);
                let items = raw
                    .into_iter()
                    .map(|item| CompletionItem {
                        label: item.label,
                        detail: item.detail,
                        filter: item.filter,
                        insert: item.insert,
                        kind: item.kind,
                        docs: item.docs,
                        snippet: item.snippet,
                        resolve: item.resolve,
                        replace: item.replace.as_ref().map(&bytes),
                        extra: item
                            .extra
                            .iter()
                            .map(|(range, text)| {
                                let (start, end) = bytes(range);
                                (start, end, text.clone())
                            })
                            .collect(),
                    })
                    .collect();
                self.emit(LspEvent::Completion {
                    token,
                    incomplete,
                    items,
                });
            }
            QueryKind::Signature => self.emit(LspEvent::Signature {
                token,
                text: plain::signature_text(result),
            }),
            QueryKind::Resolve { .. } => {
                let detail = result
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let docs = result
                    .get("documentation")
                    .map(|value| plain::hover_text(&json!({"contents": value})).unwrap_or_default())
                    .unwrap_or_default();
                self.emit(LspEvent::Resolved {
                    token,
                    detail,
                    docs,
                });
            }
            QueryKind::Rename { .. } => {
                let edits = self.patches(server, plain::workspace_edits(result), &item);
                self.emit(LspEvent::WorkspaceEdit {
                    token,
                    encoding,
                    edits,
                });
            }
            QueryKind::Format => {
                let uri = self
                    .docs
                    .get(&item.path)
                    .map(|doc| doc.uri.clone())
                    .unwrap_or_default();
                let edits = self.patches(server, plain::formatting_edits(&uri, result), &item);
                self.emit(LspEvent::WorkspaceEdit {
                    token,
                    encoding,
                    edits,
                });
            }
            QueryKind::CodeAction => {
                let actions = plain::code_actions(result)
                    .into_iter()
                    .map(|action| ActionOffer {
                        title: action.title,
                        edit: action.edit.map(|value| value.to_string()),
                        raw: action.raw,
                    })
                    .collect();
                self.emit(LspEvent::Actions {
                    token,
                    encoding,
                    actions,
                });
            }
            QueryKind::CodeActionResolve { .. } => {
                let edits = result
                    .get("edit")
                    .map(|edit| self.patches(server, plain::workspace_edits(edit), &item))
                    .unwrap_or_default();
                self.emit(LspEvent::WorkspaceEdit {
                    token,
                    encoding,
                    edits,
                });
            }
        }
    }

    /// Turns protocol edits into byte offsets in the text the server currently has.
    fn patches(
        &self,
        server: usize,
        edits: Vec<plain::RawTextEdit>,
        snap: &Inflight,
    ) -> Vec<TextPatch> {
        let encoding = self.servers[server].caps.encoding;
        edits
            .into_iter()
            .filter_map(|edit| {
                let path = uri::to_path(&edit.uri)?;
                let (version, start, end) = if path == snap.path
                    && let Some(text) = snap.text.as_ref()
                {
                    let start =
                        position::byte_of(text, edit.range.line, edit.range.character, encoding);
                    let end = position::byte_of(
                        text,
                        edit.range.end_line,
                        edit.range.end_character,
                        encoding,
                    )
                    .max(start);
                    (Some(snap.version), start, end)
                } else if let Some(doc) = self.docs.get(&path) {
                    let start = position::byte_of(
                        &doc.text,
                        edit.range.line,
                        edit.range.character,
                        encoding,
                    );
                    let end = position::byte_of(
                        &doc.text,
                        edit.range.end_line,
                        edit.range.end_character,
                        encoding,
                    )
                    .max(start);
                    (Some(doc.version), start, end)
                } else {
                    (None, 0, 0)
                };
                Some(TextPatch {
                    path,
                    version,
                    start,
                    end,
                    start_line: edit.range.line,
                    start_char: edit.range.character,
                    end_line: edit.range.end_line,
                    end_char: edit.range.end_character,
                    text: edit.text,
                })
            })
            .collect()
    }

    fn close(&mut self, path: &Path) {
        self.cancel_pull(path);
        self.cancel_tokens(path);
        let Some(doc) = self.docs.remove(path) else {
            return;
        };
        let server = &self.servers[doc.server];
        if doc.opened && server.state == State::Ready {
            server.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": doc.uri}}),
            );
        }
    }

    fn change(&mut self, path: &Path, text: Rope, version: u64, change: TextChange) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let server = &self.servers[doc.server];
        if doc.opened && server.state == State::Ready {
            let changes = match server.caps.change {
                SyncKind::None => None,
                SyncKind::Full => Some(json!([{"text": text.to_string()}])),
                SyncKind::Incremental => Some(
                    incremental_change(&doc.text, &text, change, server.caps.encoding)
                        .unwrap_or_else(|| json!([{"text": text.to_string()}])),
                ),
            };
            if let Some(changes) = changes {
                server.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": doc.uri, "version": lsp_version(version)},
                        "contentChanges": changes,
                    }),
                );
            }
        }
        doc.text = text;
        doc.version = version;
        remember(doc);
        let opened = doc.opened;
        let path = path.to_path_buf();
        if opened {
            self.schedule_pull(&path);
            self.schedule_tokens(&path);
        }
    }

    /// Launches the first installed candidate. `None` when none could start.
    fn start(&mut self, candidates: &[Arc<ServerSpec>], root: PathBuf) -> Option<usize> {
        let group = candidates[0].name.clone();
        for spec in candidates {
            match (self.launch)(spec, &root) {
                Ok(connection) => {
                    let index = self.servers.len();
                    self.servers.push(Server {
                        group: group.clone(),
                        spec: spec.clone(),
                        root: root.clone(),
                        instance: 0,
                        writer: None,
                        kill: None,
                        state: State::Starting,
                        caps: Capabilities {
                            encoding: Encoding::Utf16,
                            change: SyncKind::None,
                            save: None,
                            hover: false,
                            definition: false,
                            completion: false,
                            resolve: false,
                            signature: false,
                            rename: false,
                            format: false,
                            references: false,
                            code_action: false,
                            action_resolve: false,
                            pull: false,
                            semantic: false,
                            token_types: Vec::new(),
                            token_modifiers: Vec::new(),
                            triggers: Vec::new(),
                        },
                        next_id: 1,
                        init_id: None,
                        shutdown_id: None,
                        shutdown_acked: false,
                        restarts: 0,
                        progress: Vec::new(),
                        shown: None,
                        shown_at: None,
                        held: None,
                    });
                    self.by_root.insert((group, root), index);
                    self.connect(index, connection);
                    return Some(index);
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    self.emit(LspEvent::Notice(format!(
                        "Could not start {}: {err}",
                        spec.name
                    )));
                    return None;
                }
            }
        }
        self.missing.insert(group);
        let names: Vec<&str> = candidates
            .iter()
            .map(|spec| spec.command.as_str())
            .collect();
        self.emit(LspEvent::Notice(format!(
            "No language server found. Install {} for code diagnostics",
            names.join(" or ")
        )));
        None
    }

    /// Wires `connection` to server `index` as a new process and sends `initialize`.
    fn connect(&mut self, index: usize, connection: Connection) {
        self.next_instance += 1;
        let instance = self.next_instance;
        let (writer, kill) = attach(connection, index, instance, self.internal.clone());
        let server = &mut self.servers[index];
        server.instance = instance;
        server.writer = Some(writer);
        server.kill = Some(kill);
        server.state = State::Starting;
        server.progress.clear();
        server.shutdown_acked = false;
        let params = initialize_params(&server.root);
        server.init_id = Some(server.request("initialize", params));
        self.status(index, ServerStatus::Starting);
    }

    fn handle_internal(&mut self, message: Internal) {
        match message {
            Internal::Message {
                server,
                instance,
                body,
            } => {
                if self
                    .servers
                    .get(server)
                    .is_none_or(|slot| slot.instance != instance)
                {
                    return;
                }
                let Ok(message) = serde_json::from_slice::<Value>(&body) else {
                    return;
                };
                self.handle_message(server, message);
            }
            Internal::Exited {
                server,
                instance,
                reason,
            } => {
                if self
                    .servers
                    .get(server)
                    .is_some_and(|slot| slot.instance == instance)
                {
                    self.exited(server, reason);
                }
            }
            Internal::Flush { server } => {
                if self
                    .servers
                    .get(server)
                    .is_none_or(|slot| slot.state != State::Ready)
                {
                    return;
                }
                if let Some(status) = self.servers[server].held.take() {
                    self.status(server, status);
                }
            }
            Internal::Dispatch { token } => self.dispatch(token),
            Internal::Pull { path, generation } => self.pull(&path, generation),
            Internal::Tokens { path, generation } => self.pull_tokens(&path, generation),
            Internal::Restart { server } => {
                if self.shutting_down
                    || self
                        .servers
                        .get(server)
                        .is_none_or(|slot| slot.state != State::Restarting)
                {
                    return;
                }
                let spec = self.servers[server].spec.clone();
                let root = self.servers[server].root.clone();
                match (self.launch)(&spec, &root) {
                    Ok(connection) => self.connect(server, connection),
                    Err(err) => self.fail(server, err.to_string()),
                }
            }
        }
    }

    fn handle_message(&mut self, index: usize, message: Value) {
        let method = message["method"].as_str().map(str::to_string);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => self.answer(index, &method, id, &message["params"]),
            (Some(method), None) => self.notification(index, &method, &message["params"]),
            (None, Some(id)) => {
                let id = id.as_i64();
                let server = &mut self.servers[index];
                if id.is_some() && id == server.shutdown_id {
                    server.shutdown_acked = true;
                } else if id.is_some() && id == server.init_id {
                    server.init_id = None;
                    match message.get("error") {
                        Some(error) => {
                            let reason = error["message"]
                                .as_str()
                                .unwrap_or("initialize failed")
                                .to_string();
                            self.fail(index, reason);
                        }
                        None => self.initialized(index, &message["result"]),
                    }
                } else if let Some(rpc) = id {
                    if self.pulls.contains_key(&rpc) {
                        let result = message.get("result").cloned().unwrap_or(Value::Null);
                        self.finish_pull(index, rpc, &result);
                    } else if self.tokens.contains_key(&rpc) {
                        let result = message.get("result").cloned().unwrap_or(Value::Null);
                        self.finish_tokens(index, rpc, &result);
                    } else {
                        let token = self
                            .inflight
                            .iter()
                            .find(|item| item.server == index && item.rpc == Some(rpc))
                            .map(|item| item.token);
                        if let Some(token) = token {
                            let result = message.get("result").cloned().unwrap_or(Value::Null);
                            self.finish_query(index, token, &result);
                        }
                    }
                }
            }
            (None, None) => {}
        }
    }

    fn initialized(&mut self, index: usize, result: &Value) {
        let server = &mut self.servers[index];
        server.caps = Capabilities::parse(result);
        server.state = State::Ready;
        let caps = server.caps.clone();
        let group = server.group.clone();
        server.notify("initialized", json!({}));
        let opened = {
            let server = &self.servers[index];
            let mut opened = Vec::new();
            for (path, doc) in self.docs.iter_mut().filter(|(_, doc)| doc.server == index) {
                send_open(server, doc);
                opened.push(path.clone());
            }
            opened
        };
        for path in opened {
            self.schedule_pull(&path);
            self.schedule_tokens(&path);
        }
        self.status(index, ServerStatus::Ready);
        self.emit(LspEvent::Features {
            group,
            hover: caps.hover,
            definition: caps.definition,
            completion: caps.completion,
            triggers: caps.triggers,
            resolve: caps.resolve,
            signature: caps.signature,
            rename: caps.rename,
            format: caps.format,
            references: caps.references,
            code_action: caps.code_action,
            action_resolve: caps.action_resolve,
        });
    }

    /// Replies to a request the server sent us. Servers wait for these, so every
    /// request gets an answer, even if it is only "method not found".
    fn answer(&self, index: usize, method: &str, id: Value, params: &Value) {
        let server = &self.servers[index];
        let result = match method {
            "workspace/configuration" => {
                let name = server.spec.name.as_str();
                let items = params["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                Value::Array(
                    items
                        .iter()
                        .map(|item| configuration_item(name, item, &self.prefs))
                        .collect(),
                )
            }
            "workspace/workspaceFolders" => workspace_folders(&server.root),
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest" => Value::Null,
            "workspace/applyEdit" => json!({"applied": false}),
            _ => {
                server.send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("{method} is not supported")},
                }));
                return;
            }
        };
        server.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn notification(&mut self, index: usize, method: &str, params: &Value) {
        match method {
            "textDocument/publishDiagnostics" => self.publish(index, params),
            "$/progress" => self.progress(index, params),
            "window/showMessage" => {
                let level = params["type"].as_u64().unwrap_or(4);
                if let (1 | 2, Some(text)) = (level, params["message"].as_str()) {
                    let name = &self.servers[index].spec.name;
                    self.emit(LspEvent::Notice(format!("{name}: {}", first_line(text))));
                }
            }
            _ => {}
        }
    }

    fn publish(&self, index: usize, params: &Value) {
        let Some(target) = params["uri"].as_str() else {
            return;
        };
        let Some((path, doc)) = self
            .docs
            .iter()
            .find(|(path, doc)| doc.server == index && same_document(&doc.uri, path, target))
        else {
            return;
        };
        let encoding = self.servers[index].caps.encoding;
        let version = params["version"]
            .as_i64()
            .and_then(|version| u64::try_from(version).ok());
        let Some((version, text)) = doc.text_for(version) else {
            return;
        };
        let items = params["diagnostics"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let diagnostics = items
            .iter()
            .filter_map(|item| serde_json::from_value::<lsp_types::Diagnostic>(item.clone()).ok())
            .map(|item| convert_diagnostic(&text, item, encoding))
            .collect();
        self.emit(LspEvent::Diagnostics {
            path: path.clone(),
            version,
            diagnostics,
        });
    }

    fn progress(&mut self, index: usize, params: &Value) {
        let token = match &params["token"] {
            Value::String(token) => token.clone(),
            other => other.to_string(),
        };
        let value = &params["value"];
        let server = &mut self.servers[index];
        let existing = server.progress.iter().position(|item| item.token == token);
        match value["kind"].as_str() {
            Some("begin") => {
                let title = value["title"].as_str().unwrap_or("").to_string();
                let label = progress_label(&title, value);
                if let Some(at) = existing {
                    server.progress.remove(at);
                }
                server.progress.push(Progress {
                    token,
                    title,
                    label,
                });
            }
            Some("report") => {
                if let Some(at) = existing {
                    let item = &mut server.progress[at];
                    item.label = progress_label(&item.title, value);
                }
            }
            Some("end") => {
                if let Some(at) = existing {
                    server.progress.remove(at);
                }
            }
            _ => return,
        }
        if server.state != State::Ready {
            return;
        }
        let status = match server.progress.last() {
            Some(item) => ServerStatus::Busy(item.label.clone()),
            None => ServerStatus::Ready,
        };
        if server.shown.as_ref() == Some(&status) {
            return;
        }
        let recent = server
            .shown_at
            .is_some_and(|at| at.elapsed() < BUSY_INTERVAL);
        if matches!(status, ServerStatus::Busy(_)) && recent {
            let schedule = server.held.is_none();
            server.held = Some(status);
            if schedule {
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(BUSY_INTERVAL).await;
                    let _ = internal.send(Internal::Flush { server: index });
                });
            }
        } else {
            self.status(index, status);
        }
    }

    fn exited(&mut self, index: usize, reason: String) {
        let was_ready = self.servers[index].state == State::Ready;
        {
            let server = &mut self.servers[index];
            server.writer = None;
            server.kill = None;
            server.progress.clear();
        }
        if self.shutting_down {
            self.servers[index].state = State::Failed;
            return;
        }
        if !(was_ready && self.servers[index].restarts < MAX_RESTARTS) {
            let reason = if reason.is_empty() {
                "exited".to_string()
            } else {
                reason
            };
            self.fail(index, reason);
            return;
        }
        self.forget_documents(index);
        let server = &mut self.servers[index];
        server.state = State::Restarting;
        server.restarts += 1;
        let delay = Duration::from_secs(1 << (server.restarts - 1));
        let name = server.spec.name.clone();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = internal.send(Internal::Restart { server: index });
        });
        self.status(index, ServerStatus::Starting);
        self.emit(LspEvent::Notice(format!("{name} stopped; restarting")));
    }

    /// After the buffer settles, ask a server that offers pull diagnostics for this file.
    fn schedule_pull(&mut self, path: &Path) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let server = doc.server;
        if !doc.opened || !self.servers[server].caps.pull {
            return;
        }
        doc.pull_generation = doc.pull_generation.wrapping_add(1);
        let generation = doc.pull_generation;
        let path = path.to_path_buf();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DIAGNOSTIC_WAIT).await;
            let _ = internal.send(Internal::Pull { path, generation });
        });
    }

    fn pull(&mut self, path: &Path, generation: u64) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        if doc.pull_generation != generation || !doc.opened {
            return;
        }
        let server_index = doc.server;
        if self.servers[server_index].state != State::Ready || !self.servers[server_index].caps.pull
        {
            return;
        }
        let uri = doc.uri.clone();
        let version = doc.version;
        let text = doc.text.clone();
        self.cancel_pull(path);
        let rpc = self.servers[server_index].request(
            "textDocument/diagnostic",
            json!({"textDocument": {"uri": uri}}),
        );
        if let Some(doc) = self.docs.get_mut(path) {
            doc.pull_rpc = Some(rpc);
        }
        self.pulls.insert(
            rpc,
            PullSnap {
                path: path.to_path_buf(),
                version,
                text,
            },
        );
    }

    fn cancel_pull(&mut self, path: &Path) {
        let Some(rpc) = self.docs.get(path).and_then(|doc| doc.pull_rpc) else {
            return;
        };
        let server = self.docs.get(path).map(|doc| doc.server);
        if let Some(server) = server {
            self.servers[server].notify("$/cancelRequest", json!({"id": rpc}));
        }
        self.pulls.remove(&rpc);
        if let Some(doc) = self.docs.get_mut(path) {
            doc.pull_rpc = None;
        }
    }

    fn finish_pull(&mut self, server: usize, rpc: i64, result: &Value) {
        let Some(snap) = self.pulls.remove(&rpc) else {
            return;
        };
        if let Some(doc) = self.docs.get_mut(&snap.path)
            && doc.pull_rpc == Some(rpc)
        {
            doc.pull_rpc = None;
        }
        if result.get("kind").and_then(Value::as_str) == Some("unchanged") {
            return;
        }
        let Some(items) = result.get("items").and_then(Value::as_array) else {
            return;
        };
        let encoding = self.servers[server].caps.encoding;
        let diagnostics = items
            .iter()
            .filter_map(|item| serde_json::from_value::<lsp_types::Diagnostic>(item.clone()).ok())
            .map(|item| convert_diagnostic(&snap.text, item, encoding))
            .collect();
        self.emit(LspEvent::PulledDiagnostics {
            path: snap.path,
            version: snap.version,
            diagnostics,
        });
    }

    fn schedule_tokens(&mut self, path: &Path) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let server = doc.server;
        if !doc.opened || !self.servers[server].caps.semantic {
            return;
        }
        doc.token_generation = doc.token_generation.wrapping_add(1);
        let generation = doc.token_generation;
        let path = path.to_path_buf();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DIAGNOSTIC_WAIT).await;
            let _ = internal.send(Internal::Tokens { path, generation });
        });
    }

    fn pull_tokens(&mut self, path: &Path, generation: u64) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        if doc.token_generation != generation || !doc.opened {
            return;
        }
        let server_index = doc.server;
        if self.servers[server_index].state != State::Ready
            || !self.servers[server_index].caps.semantic
        {
            return;
        }
        let uri = doc.uri.clone();
        let version = doc.version;
        let text = doc.text.clone();
        self.cancel_tokens(path);
        let rpc = self.servers[server_index].request(
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": uri}}),
        );
        if let Some(doc) = self.docs.get_mut(path) {
            doc.token_rpc = Some(rpc);
        }
        self.tokens.insert(
            rpc,
            PullSnap {
                path: path.to_path_buf(),
                version,
                text,
            },
        );
    }

    fn cancel_tokens(&mut self, path: &Path) {
        let Some(rpc) = self.docs.get(path).and_then(|doc| doc.token_rpc) else {
            return;
        };
        let server = self.docs.get(path).map(|doc| doc.server);
        if let Some(server) = server {
            self.servers[server].notify("$/cancelRequest", json!({"id": rpc}));
        }
        self.tokens.remove(&rpc);
        if let Some(doc) = self.docs.get_mut(path) {
            doc.token_rpc = None;
        }
    }

    fn finish_tokens(&mut self, server: usize, rpc: i64, result: &Value) {
        let Some(snap) = self.tokens.remove(&rpc) else {
            return;
        };
        if let Some(doc) = self.docs.get_mut(&snap.path)
            && doc.token_rpc == Some(rpc)
        {
            doc.token_rpc = None;
        }
        let Some(data) = result.get("data").and_then(Value::as_array) else {
            return;
        };
        let data: Vec<u64> = data.iter().filter_map(Value::as_u64).collect();
        let caps = &self.servers[server].caps;
        let spans = crate::lsp::semantic::decode(
            &data,
            &snap.text,
            caps.encoding,
            &caps.token_types,
            &caps.token_modifiers,
        );
        self.emit(LspEvent::SemanticTokens {
            path: snap.path,
            version: snap.version,
            spans,
        });
    }

    /// The process is gone: its diagnostics are stale, and a restarted process needs `didOpen` again.
    fn forget_documents(&mut self, index: usize) {
        let paths: Vec<PathBuf> = self
            .docs
            .iter()
            .filter(|(_, doc)| doc.server == index)
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            self.cancel_pull(&path);
            self.cancel_tokens(&path);
        }
        let stale: Vec<u64> = self
            .inflight
            .iter()
            .filter(|item| item.server == index)
            .map(|item| item.token)
            .collect();
        for token in stale {
            if let Some(pos) = self.inflight.iter().position(|item| item.token == token) {
                let item = self.inflight.remove(pos);
                self.empty_query(item.token, &item.kind);
            }
        }
        for (path, doc) in self.docs.iter_mut().filter(|(_, doc)| doc.server == index) {
            doc.opened = false;
            let _ = self.events.send(LspEvent::Diagnostics {
                path: path.clone(),
                version: doc.version,
                diagnostics: Vec::new(),
            });
        }
    }

    fn fail(&mut self, index: usize, reason: String) {
        let server = &mut self.servers[index];
        server.state = State::Failed;
        server.writer = None;
        server.kill = None;
        let name = server.spec.name.clone();
        self.forget_documents(index);
        self.status(index, ServerStatus::Failed(reason.clone()));
        self.emit(LspEvent::Notice(format!(
            "{name} stopped: {}",
            first_line(&reason)
        )));
    }

    /// Stops every running copy of the server called `name` (one per project root),
    /// as the user asked in Settings. Its documents are forgotten; `App` opens them
    /// again, which starts the next server for them.
    fn stop_named(&mut self, name: &str) {
        let targets: Vec<usize> = (0..self.servers.len())
            .filter(|&index| {
                self.servers[index].spec.name == name && self.servers[index].state != State::Failed
            })
            .collect();
        for index in targets {
            self.stop_server(index);
        }
    }

    fn stop_server(&mut self, index: usize) {
        {
            let server = &mut self.servers[index];
            if server.writer.is_some() {
                if server.state == State::Ready {
                    server.request("shutdown", Value::Null);
                }
                server.notify("exit", Value::Null);
            }
            // Queued messages are still written; the process is killed if it lingers.
            server.writer = None;
            if let Some(kill) = server.kill.take() {
                tokio::spawn(async move {
                    tokio::time::sleep(EXIT_WAIT).await;
                    drop(kill);
                });
            }
            server.state = State::Failed;
            server.progress.clear();
        }
        // Events from the old process no longer match this server.
        self.next_instance += 1;
        self.servers[index].instance = self.next_instance;
        self.forget_documents(index);
        self.docs.retain(|_, doc| doc.server != index);
        self.inflight.retain(|item| item.server != index);
        self.by_root.retain(|_, server| *server != index);
        let server = &self.servers[index];
        self.emit(LspEvent::Stopped {
            group: server.group.clone(),
            name: server.spec.name.clone(),
        });
    }

    /// Drop every server after [`Self::shutdown`] so a new folder can start its own.
    fn clear_after_reset(&mut self) {
        self.shutting_down = false;
        self.servers.clear();
        self.by_root.clear();
        self.docs.clear();
        self.inflight.clear();
        self.pulls.clear();
        self.tokens.clear();
    }

    /// `shutdown`, then `exit`, then kill whatever is still running.
    async fn shutdown(&mut self, internal_rx: &mut mpsc::UnboundedReceiver<Internal>) {
        self.shutting_down = true;
        let live: Vec<usize> = (0..self.servers.len())
            .filter(|&index| self.servers[index].writer.is_some())
            .collect();
        for &index in &live {
            let server = &mut self.servers[index];
            if server.state == State::Ready {
                server.shutdown_id = Some(server.request("shutdown", Value::Null));
            } else {
                server.shutdown_acked = true;
            }
        }
        let acked = |manager: &Manager| {
            live.iter().all(|&index| {
                manager.servers[index].shutdown_acked || manager.servers[index].writer.is_none()
            })
        };
        let _ = tokio::time::timeout(SHUTDOWN_REPLY, async {
            while !acked(self) {
                match internal_rx.recv().await {
                    Some(message) => self.handle_internal(message),
                    None => break,
                }
            }
        })
        .await;
        for &index in &live {
            self.servers[index].notify("exit", Value::Null);
        }
        let _ = tokio::time::timeout(EXIT_WAIT, async {
            while live
                .iter()
                .any(|&index| self.servers[index].writer.is_some())
            {
                match internal_rx.recv().await {
                    Some(message) => self.handle_internal(message),
                    None => break,
                }
            }
        })
        .await;
        for server in &mut self.servers {
            server.writer = None;
            server.kill = None;
        }
    }
}

fn send_open(server: &Server, doc: &mut Doc) {
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": doc.uri,
            "languageId": doc.language_id,
            "version": lsp_version(doc.version),
            "text": doc.text.to_string(),
        }}),
    );
    doc.opened = true;
}

/// One range edit, or `None` when the span does not fit the texts and the whole file must be sent.
fn incremental_change(
    old: &Rope,
    new: &Rope,
    change: TextChange,
    encoding: Encoding,
) -> Option<Value> {
    let TextChange::Range {
        start,
        old_end,
        new_end,
    } = change
    else {
        return None;
    };
    let fits = start <= old_end
        && start <= new_end
        && old_end <= old.len_bytes()
        && old.len_bytes() - (old_end - start) + (new_end - start) == new.len_bytes();
    if !fits {
        return None;
    }
    let inserted = new.get_byte_slice(start..new_end)?.to_string();
    old.get_byte_slice(start..old_end)?;
    let (start_line, start_char) = position::position_of(old, start, encoding);
    let (end_line, end_char) = position::position_of(old, old_end, encoding);
    Some(json!([{
        "range": {
            "start": {"line": start_line, "character": start_char},
            "end": {"line": end_line, "character": end_char},
        },
        "text": inserted,
    }]))
}

fn convert_diagnostic(text: &Rope, item: lsp_types::Diagnostic, encoding: Encoding) -> Diagnostic {
    let range = item.range;
    let start = position::byte_of(text, range.start.line, range.start.character, encoding);
    let end = position::byte_of(text, range.end.line, range.end.character, encoding).max(start);
    let code = item.code.map(|code| match code {
        lsp_types::NumberOrString::Number(number) => number.to_string(),
        lsp_types::NumberOrString::String(text) => text,
    });
    Diagnostic {
        start,
        end,
        severity: Severity::from_lsp(item.severity),
        message: item.message,
        source: item.source,
        code,
    }
}

/// Servers may spell a URI differently than we sent it (`e%3A` for `E:`), so fall back to comparing paths.
fn same_document(sent: &str, path: &Path, received: &str) -> bool {
    if sent == received {
        return true;
    }
    let Some(received) = uri::to_path(received) else {
        return false;
    };
    if cfg!(windows) {
        received.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase()
    } else {
        received == path
    }
}

fn lsp_version(version: u64) -> i64 {
    version.min(i32::MAX as u64) as i64
}

fn progress_label(title: &str, value: &Value) -> String {
    let mut label = title.to_string();
    if let Some(message) = value["message"].as_str().filter(|m| !m.is_empty()) {
        label.push_str(" — ");
        label.push_str(first_line(message));
    }
    if let Some(percent) = value["percentage"].as_u64() {
        label.push_str(&format!(" {percent}%"));
    }
    label
}

fn same_feature(left: &QueryKind, right: &QueryKind) -> bool {
    matches!(
        (left, right),
        (QueryKind::Hover, QueryKind::Hover)
            | (QueryKind::Definition, QueryKind::Definition)
            | (QueryKind::Completion { .. }, QueryKind::Completion { .. })
            | (QueryKind::Signature, QueryKind::Signature)
            | (QueryKind::Resolve { .. }, QueryKind::Resolve { .. })
            | (QueryKind::Rename { .. }, QueryKind::Rename { .. })
            | (QueryKind::Format, QueryKind::Format)
            | (QueryKind::References, QueryKind::References)
            | (QueryKind::CodeAction, QueryKind::CodeAction)
            | (
                QueryKind::CodeActionResolve { .. },
                QueryKind::CodeActionResolve { .. }
            )
    )
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

/// One entry of a `workspace/configuration` result.
///
/// Pyright and basedpyright default to `typeCheckingMode: "off"` when this is null,
/// which reports syntax errors only. `basic` also reports an undefined name, and
/// `openFilesOnly` keeps the check on the files the editor sent.
fn configuration_item(server: &str, item: &Value, prefs: &crate::settings::LspPrefs) -> Value {
    let section = item["section"].as_str().unwrap_or("");
    let analysis = json!({
        "typeCheckingMode": prefs.python.as_str(),
        "diagnosticMode": "openFilesOnly",
    });
    if server == "rust-analyzer" && section == "rust-analyzer" && !prefs.check_on_save {
        return json!({"checkOnSave": false});
    }
    match (server, section) {
        ("pyright" | "basedpyright", "python") => json!({ "analysis": analysis }),
        ("pyright" | "basedpyright", "python.analysis") => analysis,
        ("basedpyright", "basedpyright") => json!({ "analysis": analysis }),
        ("basedpyright", "basedpyright.analysis") => analysis,
        _ => Value::Null,
    }
}

fn workspace_folders(root: &Path) -> Value {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    json!([{"uri": uri::from_path(root), "name": name}])
}

fn initialize_params(root: &Path) -> Value {
    json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "duckx-code", "version": env!("CARGO_PKG_VERSION")},
        "rootPath": root.to_string_lossy(),
        "rootUri": uri::from_path(root),
        "workspaceFolders": workspace_folders(root),
        "capabilities": {
            "general": {"positionEncodings": ["utf-8", "utf-16"]},
            "textDocument": {
                "synchronization": {"dynamicRegistration": false, "didSave": true},
                "publishDiagnostics": {"relatedInformation": false, "versionSupport": true},
                "diagnostic": {"dynamicRegistration": false, "relatedDocumentSupport": false},
                "rename": {"dynamicRegistration": false, "prepareSupport": false},
                "formatting": {"dynamicRegistration": false},
                "codeAction": {
                    "dynamicRegistration": false,
                    "codeActionLiteralSupport": {
                        "codeActionKind": {"valueSet": ["", "quickfix", "refactor", "source"]},
                    },
                    "resolveSupport": {"properties": ["edit"]},
                },
                "hover": {"contentFormat": ["plaintext", "markdown"]},
                "definition": {"linkSupport": true},
                "references": {"dynamicRegistration": false},
                "completion": {
                    "completionItem": {
                        "snippetSupport": true,
                        "documentationFormat": ["plaintext", "markdown"],
                        "resolveSupport": {"properties": ["documentation", "detail"]},
                    },
                    "contextSupport": true,
                },
                "signatureHelp": {
                    "signatureInformation": {
                        "documentationFormat": ["plaintext", "markdown"],
                    },
                },
                "semanticTokens": {
                    "requests": {"full": true},
                    "tokenTypes": [
                        "namespace", "type", "class", "enum", "interface", "struct",
                        "typeParameter", "parameter", "variable", "property", "enumMember",
                        "function", "method", "macro", "keyword", "modifier", "comment",
                        "string", "number", "regexp", "operator", "decorator", "lifetime"
                    ],
                    "tokenModifiers": ["declaration", "documentation", "defaultLibrary", "static"],
                    "formats": ["relative"],
                },
            },
            "window": {"workDoneProgress": true},
            "workspace": {"workspaceFolders": true, "configuration": true},
        },
    })
}

/// Starts the reader, writer, stderr, and process tasks for one connection.
fn attach(
    connection: Connection,
    server: usize,
    instance: u64,
    internal: mpsc::UnboundedSender<Internal>,
) -> (mpsc::UnboundedSender<Value>, oneshot::Sender<()>) {
    let Connection {
        mut input,
        output,
        stderr,
        child,
        log,
    } = connection;

    let (writer, mut outgoing) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            if transport::write_message(&mut input, &message)
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = input.shutdown().await;
    });

    let stderr_task = stderr.map(|pipe| tokio::spawn(drain_stderr(pipe, log)));

    tokio::spawn(async move {
        let mut output = BufReader::new(output);
        while let Ok(Some(body)) = transport::read_body(&mut output).await {
            if internal
                .send(Internal::Message {
                    server,
                    instance,
                    body,
                })
                .is_err()
            {
                return;
            }
        }
        let reason = match stderr_task {
            Some(task) => tokio::time::timeout(STDERR_TAIL_WAIT, task)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default(),
            None => String::new(),
        };
        let _ = internal.send(Internal::Exited {
            server,
            instance,
            reason,
        });
    });

    let (kill, kill_rx) = oneshot::channel::<()>();
    if let Some(mut child) = child {
        tokio::spawn(async move {
            tokio::select! {
                _ = child.wait() => {}
                _ = kill_rx => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
        });
    }
    (writer, kill)
}

/// Copies stderr to the log file and returns its last non-empty line.
async fn drain_stderr(pipe: Box<dyn AsyncRead + Send + Unpin>, log: Option<PathBuf>) -> String {
    let mut file = match log {
        Some(path) => {
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            tokio::fs::File::create(path).await.ok()
        }
        None => None,
    };
    let mut lines = BufReader::new(pipe).lines();
    let mut last = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(file) = file.as_mut() {
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
        if !line.trim().is_empty() {
            last = line.trim().to_string();
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Language;
    use std::sync::Mutex;

    /// The built-in servers for `language`'s files, in fallback order.
    fn specs(language: Language) -> Vec<Arc<ServerSpec>> {
        let file = if language == Language::Rust {
            "x.rs"
        } else {
            "x.py"
        };
        registry::Registry::default().candidates(Path::new(file))
    }

    /// One sender per launched fake server; a test sends raw messages "from the server" through it.
    type Injectors = Arc<Mutex<Vec<mpsc::UnboundedSender<Value>>>>;

    /// An in-process server: answers `initialize`, publishes one diagnostic per
    /// `didOpen`, and forwards every message it receives to the test.
    fn fake_launcher(caps: Value) -> (Launch, mpsc::UnboundedReceiver<Value>, Injectors) {
        let (seen_tx, seen_rx) = mpsc::unbounded_channel();
        let injectors = Arc::new(Mutex::new(Vec::new()));
        let injectors_out = injectors.clone();
        let launch: Launch = Arc::new(move |_spec, _root| {
            let (client_in, server_in) = tokio::io::duplex(1 << 16);
            let (server_out, client_out) = tokio::io::duplex(1 << 16);
            let (inject_tx, mut inject_rx) = mpsc::unbounded_channel::<Value>();
            injectors.lock().unwrap().push(inject_tx);
            let seen = seen_tx.clone();
            let caps = caps.clone();
            let (incoming_tx, mut incoming) = mpsc::unbounded_channel::<Value>();
            tokio::spawn(async move {
                let mut reader = BufReader::new(server_in);
                while let Ok(Some(body)) = transport::read_body(&mut reader).await {
                    let _ = incoming_tx.send(serde_json::from_slice(&body).unwrap());
                }
            });
            tokio::spawn(async move {
                let mut writer = server_out;
                loop {
                    tokio::select! {
                        message = incoming.recv() => {
                            let Some(message) = message else { return };
                            let _ = seen.send(message.clone());
                            match message["method"].as_str() {
                                Some("initialize") => {
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": caps}});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("textDocument/hover") => {
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": {"contents": {"kind": "markdown", "value": "**hello**"}}});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("textDocument/definition") => {
                                    let uri = message["params"]["textDocument"]["uri"].clone();
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": {"uri": uri, "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}}}});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("textDocument/references") => {
                                    let uri = message["params"]["textDocument"]["uri"].clone();
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": [
                                        {"uri": uri, "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}}},
                                        {"uri": uri, "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 1}}}
                                    ]});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("textDocument/completion") => {
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": {"isIncomplete": false, "items": [{"label": "println", "insertTextFormat": 2, "insertText": "println!($0)"}]}});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("textDocument/didOpen") => {
                                    let uri = message["params"]["textDocument"]["uri"].clone();
                                    let publish = json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                                        "uri": uri,
                                        "diagnostics": [{
                                            "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}},
                                            "severity": 2,
                                            "message": "unused variable",
                                            "source": "fake",
                                            "code": 7,
                                        }],
                                    }});
                                    transport::write_message(&mut writer, &publish).await.unwrap();
                                }
                                Some("shutdown") => {
                                    let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": null});
                                    transport::write_message(&mut writer, &reply).await.unwrap();
                                }
                                Some("exit") => return,
                                _ => {}
                            }
                        }
                        Some(extra) = inject_rx.recv() => {
                            transport::write_message(&mut writer, &extra).await.unwrap();
                        }
                    }
                }
            });
            Ok(Connection {
                input: Box::new(client_in),
                output: Box::new(client_out),
                stderr: None,
                child: None,
                log: None,
            })
        });
        (launch, seen_rx, injectors_out)
    }

    async fn next<T>(rx: &mut mpsc::UnboundedReceiver<T>, mut wanted: impl FnMut(&T) -> bool) -> T {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let item = rx.recv().await.expect("channel open");
                if wanted(&item) {
                    return item;
                }
            }
        })
        .await
        .expect("message arrives")
    }

    /// Runs only when `pyright-langserver` is on `PATH`. Covers the real server's
    /// configuration request and its percent-encoded Windows URI.
    #[tokio::test]
    async fn installed_pyright_reports_an_undefined_name() {
        let found = std::process::Command::new("where")
            .arg("pyright-langserver")
            .output()
            .is_ok_and(|out| out.status.success());
        if !found {
            return;
        }
        let dir = std::env::temp_dir().join("duckx-pyright-probe");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.py");
        let (handle, mut events) = spawn(None);
        handle.send(LspRequest::Open {
            path,
            servers: specs(Language::Python),
            text: Rope::from_str("prrrrrint('Hello, world!')\n"),
            version: 1,
            workspace: dir,
        });
        let diagnostics = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                match events.recv().await {
                    Some(LspEvent::Diagnostics { diagnostics, .. }) if !diagnostics.is_empty() => {
                        return diagnostics;
                    }
                    Some(_) => {}
                    None => panic!("language server closed the channel"),
                }
            }
        })
        .await
        .expect("pyright publishes a diagnostic");
        assert!(
            diagnostics
                .iter()
                .any(|item| item.message.contains("prrrrrint")),
            "{diagnostics:?}"
        );
        handle.shutdown(Duration::from_secs(5)).await;
    }

    #[test]
    fn basedpyright_configuration_uses_the_same_checking_mode() {
        let analysis = json!({"typeCheckingMode": "basic", "diagnosticMode": "openFilesOnly"});
        assert_eq!(
            configuration_item(
                "basedpyright",
                &json!({"section": "basedpyright.analysis"}),
                &crate::settings::LspPrefs::default(),
            ),
            analysis
        );
        assert_eq!(
            configuration_item(
                "basedpyright",
                &json!({"section": "python"}),
                &crate::settings::LspPrefs::default(),
            ),
            json!({"analysis": analysis})
        );
        assert_eq!(
            configuration_item(
                "pylsp",
                &json!({"section": "python.analysis"}),
                &crate::settings::LspPrefs::default(),
            ),
            Value::Null
        );
    }

    fn program_installed(name: &str) -> bool {
        std::process::Command::new("where")
            .arg(name)
            .output()
            .is_ok_and(|out| out.status.success())
    }

    #[tokio::test]
    async fn installed_language_servers_answer_hover_definition_and_completion() {
        if program_installed("rust-analyzer") {
            probe_server(
                Language::Rust,
                "fn work() {}\nfn main() {\n    work();\n}\n",
                "src/main.rs",
                &["Cargo.toml"],
                "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .await;
        }
        if program_installed("pyright-langserver") {
            probe_server(
                Language::Python,
                "def hello():\n    return 1\n\nhello()\n",
                "hello.py",
                &[],
                "",
            )
            .await;
        }
    }

    async fn probe_server(
        language: Language,
        text: &str,
        file_name: &str,
        markers: &[&str],
        marker_text: &str,
    ) {
        let dir =
            std::env::temp_dir().join(format!("duckx-lsp-probe-{}", file_name.replace('.', "-")));
        std::fs::create_dir_all(&dir).unwrap();
        for marker in markers {
            std::fs::write(dir.join(marker), marker_text).unwrap();
        }
        let path = dir.join(file_name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let (handle, mut events) = spawn(None);
        handle.send(LspRequest::Open {
            path: path.clone(),
            servers: specs(language),
            text: Rope::from_str(text),
            version: 1,
            workspace: dir,
        });
        let features = tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                match events.recv().await {
                    Some(event @ LspEvent::Features { .. }) => return event,
                    Some(LspEvent::Status {
                        status: ServerStatus::Failed(reason),
                        ..
                    }) => {
                        panic!("server failed: {reason}");
                    }
                    Some(_) => {}
                    None => panic!("language server closed the channel"),
                }
            }
        })
        .await
        .expect("server publishes its features");
        let LspEvent::Features {
            hover,
            definition,
            completion,
            ..
        } = features
        else {
            panic!("features");
        };
        assert!(hover && definition && completion, "{features:?}");

        let at = text
            .rfind(if language == Language::Rust {
                "work"
            } else {
                "hello"
            })
            .unwrap();
        let mut token = 10u64;
        let hover = retry(
            &handle,
            &mut events,
            &mut token,
            |token| LspRequest::Query {
                token,
                path: path.clone(),
                at,
                kind: QueryKind::Hover,
            },
            |event, token| match event {
                LspEvent::Hover { token: got, text } if *got == token => Some(text.clone()),
                _ => None,
            },
            |text| text.as_deref().is_some_and(|text| !text.is_empty()),
        )
        .await;
        assert!(
            hover.as_deref().is_some_and(|text| !text.is_empty()),
            "hover was empty for {path:?}: {hover:?}"
        );

        let locations = retry(
            &handle,
            &mut events,
            &mut token,
            |token| LspRequest::Query {
                token,
                path: path.clone(),
                at,
                kind: QueryKind::Definition,
            },
            |event, token| match event {
                LspEvent::Definition {
                    token: got,
                    locations,
                } if *got == token => Some(locations.clone()),
                _ => None,
            },
            |locations| !locations.is_empty(),
        )
        .await;
        assert!(!locations.is_empty(), "no definition for {path:?}");

        let items = retry(
            &handle,
            &mut events,
            &mut token,
            |token| LspRequest::Query {
                token,
                path: path.clone(),
                at,
                kind: QueryKind::Completion {
                    trigger: CompletionTrigger::Invoked,
                    immediate: true,
                },
            },
            |event, token| match event {
                LspEvent::Completion {
                    token: got, items, ..
                } if *got == token => Some(items.clone()),
                _ => None,
            },
            |items| !items.is_empty(),
        )
        .await;
        assert!(!items.is_empty(), "completion was empty for {path:?}");
        handle.shutdown(Duration::from_secs(5)).await;
    }

    async fn retry<T: Clone>(
        handle: &LspHandle,
        events: &mut mpsc::UnboundedReceiver<LspEvent>,
        token: &mut u64,
        request: impl Fn(u64) -> LspRequest,
        take: impl Fn(&LspEvent, u64) -> Option<T>,
        done: impl Fn(&T) -> bool,
    ) -> T {
        let started = Instant::now();
        let mut last = None;
        while started.elapsed() < Duration::from_secs(25) {
            *token += 1;
            let current = *token;
            handle.send(request(current));
            let reply = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match events.recv().await {
                        Some(event) => {
                            if let Some(value) = take(&event, current) {
                                return value;
                            }
                        }
                        None => panic!("language server closed the channel"),
                    }
                }
            })
            .await;
            if let Ok(value) = reply {
                if done(&value) {
                    return value;
                }
                last = Some(value);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        last.expect("server never answered")
    }

    #[tokio::test]
    async fn a_diagnostic_for_an_older_version_is_not_read_as_the_new_text() {
        let (launch, mut seen, injectors) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, mut events) = spawn_with(launch);
        let path = PathBuf::from("/w/src/main.rs");
        handle.send(open(&path, "hello\n"));
        let opened = next(&mut seen, method("textDocument/didOpen")).await;
        let uri = opened["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .to_string();
        next(&mut events, |event| {
            matches!(event, LspEvent::Diagnostics { .. })
        })
        .await;
        handle.send(LspRequest::Change {
            path: path.clone(),
            text: Rope::from_str("Xhello\n"),
            version: 2,
            change: TextChange::Range {
                start: 0,
                old_end: 0,
                new_end: 1,
            },
        });
        next(&mut seen, method("textDocument/didChange")).await;
        let inject = injectors.lock().unwrap()[0].clone();
        inject
            .send(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": uri,
                    "version": 1,
                    "diagnostics": [{
                        "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}},
                        "message": "o",
                    }],
                },
            }))
            .unwrap();
        let event = next(&mut events, |event| {
            matches!(event, LspEvent::Diagnostics { version: 1, .. })
        })
        .await;
        let LspEvent::Diagnostics {
            version,
            diagnostics,
            ..
        } = event
        else {
            panic!("diagnostic");
        };
        assert_eq!(version, 1);
        assert_eq!(diagnostics[0].start, 4);
        handle.shutdown(Duration::from_secs(5)).await;
    }

    fn method(name: &'static str) -> impl FnMut(&Value) -> bool {
        move |message| message["method"] == name
    }

    fn open(path: &Path, text: &str) -> LspRequest {
        LspRequest::Open {
            path: path.to_path_buf(),
            servers: specs(Language::Rust),
            text: Rope::from_str(text),
            version: 1,
            workspace: PathBuf::from("/w"),
        }
    }

    #[tokio::test]
    async fn open_change_diagnostics_and_shutdown() {
        let (launch, mut seen, _) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, mut events) = spawn_with(launch);
        let path = PathBuf::from("/w/src/main.rs");
        handle.send(open(&path, "let x = 1;\n"));

        let init = next(&mut seen, method("initialize")).await;
        assert_eq!(
            init["params"]["capabilities"]["general"]["positionEncodings"][0],
            "utf-8"
        );
        next(&mut seen, method("initialized")).await;
        let opened = next(&mut seen, method("textDocument/didOpen")).await;
        assert_eq!(opened["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(opened["params"]["textDocument"]["text"], "let x = 1;\n");

        let event = next(&mut events, |event| {
            matches!(event, LspEvent::Diagnostics { .. })
        })
        .await;
        let LspEvent::Diagnostics {
            path: got,
            version,
            diagnostics,
        } = event
        else {
            unreachable!()
        };
        assert_eq!((got, version), (path.clone(), 1));
        assert_eq!((diagnostics[0].start, diagnostics[0].end), (4, 5));
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert_eq!(diagnostics[0].code.as_deref(), Some("7"));

        // The server did not pick an encoding, so columns are UTF-16 units: the duck is two.
        handle.send(LspRequest::Change {
            path: path.clone(),
            text: Rope::from_str("🦆let x = 1;\n"),
            version: 2,
            change: TextChange::Range {
                start: 0,
                old_end: 0,
                new_end: 4,
            },
        });
        handle.send(LspRequest::Change {
            path: path.clone(),
            text: Rope::from_str("🦆alet x = 1;\n"),
            version: 3,
            change: TextChange::Range {
                start: 4,
                old_end: 4,
                new_end: 5,
            },
        });
        let first = next(&mut seen, method("textDocument/didChange")).await;
        assert_eq!(first["params"]["contentChanges"][0]["text"], "🦆");
        let second = next(&mut seen, method("textDocument/didChange")).await;
        assert_eq!(second["params"]["textDocument"]["version"], 3);
        assert_eq!(
            second["params"]["contentChanges"][0]["range"]["start"]["character"],
            2
        );
        assert_eq!(second["params"]["contentChanges"][0]["text"], "a");

        handle.shutdown(Duration::from_secs(5)).await;
        next(&mut seen, method("shutdown")).await;
        next(&mut seen, method("exit")).await;
    }

    #[tokio::test]
    async fn documents_opened_during_initialize_are_sent_with_their_latest_text() {
        let (launch, mut seen, _) =
            fake_launcher(json!({"textDocumentSync": 1, "positionEncoding": "utf-8"}));
        let (handle, _events) = spawn_with(launch);
        let path = PathBuf::from("/w/src/lib.rs");
        handle.send(open(&path, "a"));
        handle.send(LspRequest::Change {
            path: path.clone(),
            text: Rope::from_str("ab"),
            version: 2,
            change: TextChange::Range {
                start: 1,
                old_end: 1,
                new_end: 2,
            },
        });
        let opened = next(&mut seen, method("textDocument/didOpen")).await;
        assert_eq!(opened["params"]["textDocument"]["text"], "ab");
        assert_eq!(opened["params"]["textDocument"]["version"], 2);

        // Full sync sends the whole text.
        handle.send(LspRequest::Change {
            path: path.clone(),
            text: Rope::from_str("abc"),
            version: 3,
            change: TextChange::Range {
                start: 2,
                old_end: 2,
                new_end: 3,
            },
        });
        let changed = next(&mut seen, method("textDocument/didChange")).await;
        assert_eq!(
            changed["params"]["contentChanges"][0],
            json!({"text": "abc"})
        );
        handle.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn hover_definition_and_completion_come_back_as_plain_text() {
        let (launch, mut seen) = {
            let caps = json!({
                "textDocumentSync": 1,
                "hoverProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "completionProvider": {"triggerCharacters": ["."]},
            });
            let (launch, seen, _) = fake_launcher(caps);
            (launch, seen)
        };
        let (handle, mut events) = spawn_with(launch);
        let path = PathBuf::from("/w/src/main.rs");
        handle.send(open(&path, "let x = 1;\n"));
        let features = next_event(&mut events, |event| {
            matches!(event, LspEvent::Features { .. })
        })
        .await;
        let LspEvent::Features {
            hover,
            definition,
            completion,
            triggers,
            ..
        } = features
        else {
            panic!("features");
        };
        assert!(hover && definition && completion);
        assert_eq!(triggers, vec![".".to_string()]);
        next(&mut seen, method("textDocument/didOpen")).await;

        handle.send(LspRequest::Query {
            token: 1,
            path: path.clone(),
            at: 4,
            kind: QueryKind::Hover,
        });
        next(&mut seen, method("textDocument/hover")).await;
        let hover = next_event(&mut events, |event| matches!(event, LspEvent::Hover { .. })).await;
        assert_eq!(
            hover,
            LspEvent::Hover {
                token: 1,
                text: Some("hello".into())
            }
        );

        handle.send(LspRequest::Query {
            token: 2,
            path: path.clone(),
            at: 4,
            kind: QueryKind::Definition,
        });
        next(&mut seen, method("textDocument/definition")).await;
        let definition = next_event(&mut events, |event| {
            matches!(event, LspEvent::Definition { .. })
        })
        .await;
        let LspEvent::Definition { token, locations } = definition else {
            panic!("definition");
        };
        assert_eq!(token, 2);
        assert_eq!(locations.len(), 1);
        assert_eq!((locations[0].line, locations[0].character), (0, 4));

        handle.send(LspRequest::Query {
            token: 4,
            path: path.clone(),
            at: 4,
            kind: QueryKind::References,
        });
        let request = next(&mut seen, method("textDocument/references")).await;
        assert_eq!(request["params"]["context"]["includeDeclaration"], true);
        let references = next_event(&mut events, |event| {
            matches!(event, LspEvent::References { .. })
        })
        .await;
        let LspEvent::References { token, locations } = references else {
            panic!("references");
        };
        assert_eq!(token, 4);
        assert_eq!(locations.len(), 2);
        assert_eq!((locations[1].line, locations[1].character), (1, 0));

        handle.send(LspRequest::Query {
            token: 3,
            path: path.clone(),
            at: 4,
            kind: QueryKind::Completion {
                trigger: CompletionTrigger::Invoked,
                immediate: true,
            },
        });
        let request = next(&mut seen, method("textDocument/completion")).await;
        assert_eq!(request["params"]["context"]["triggerKind"], 1);
        let completion = next_event(&mut events, |event| {
            matches!(event, LspEvent::Completion { .. })
        })
        .await;
        let LspEvent::Completion { items, .. } = completion else {
            panic!("completion");
        };
        assert_eq!(items[0].insert, "println!($0)");
        assert!(items[0].snippet);
        handle.shutdown(Duration::from_secs(5)).await;
    }

    async fn next_event(
        rx: &mut mpsc::UnboundedReceiver<LspEvent>,
        mut wanted: impl FnMut(&LspEvent) -> bool,
    ) -> LspEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let item = rx.recv().await.expect("channel open");
                if wanted(&item) {
                    return item;
                }
            }
        })
        .await
        .expect("event arrives")
    }

    #[tokio::test]
    async fn server_requests_get_answers() {
        let (launch, mut seen, injectors) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, _events) = spawn_with(launch);
        handle.send(open(Path::new("/w/a.rs"), ""));
        next(&mut seen, method("textDocument/didOpen")).await;
        let inject = injectors.lock().unwrap()[0].clone();
        inject
            .send(json!({"jsonrpc": "2.0", "id": 41, "method": "workspace/configuration", "params": {"items": [{}, {}]}}))
            .unwrap();
        inject
            .send(json!({"jsonrpc": "2.0", "id": "x", "method": "made/up", "params": {}}))
            .unwrap();
        let config = next(&mut seen, |message| message["id"] == 41).await;
        assert_eq!(config["result"], json!([null, null]));
        let unknown = next(&mut seen, |message| message["id"] == "x").await;
        assert_eq!(unknown["error"]["code"], -32601);
        handle.shutdown(Duration::from_secs(5)).await;

        let (launch, mut seen, injectors) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, _events) = spawn_with(launch);
        handle.send(LspRequest::Open {
            path: PathBuf::from("/w/hello.py"),
            servers: specs(Language::Python),
            text: Rope::from_str("prrint('x')\n"),
            version: 1,
            workspace: PathBuf::from("/w"),
        });
        next(&mut seen, method("textDocument/didOpen")).await;
        let inject = injectors.lock().unwrap()[0].clone();
        inject
            .send(json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "workspace/configuration",
                "params": {"items": [
                    {"section": "python"},
                    {"section": "python.analysis"},
                    {"section": "python.pythonPath"},
                    {"section": "basedpyright.analysis"},
                ]},
            }))
            .unwrap();
        let config = next(&mut seen, |message| message["id"] == 7).await;
        let analysis = json!({"typeCheckingMode": "basic", "diagnosticMode": "openFilesOnly"});
        assert_eq!(
            config["result"],
            json!([{"analysis": analysis}, analysis, null, null])
        );
        handle.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn progress_and_missing_programs_reach_the_status_bar() {
        let (launch, mut seen, injectors) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, mut events) = spawn_with(launch);
        handle.send(open(Path::new("/w/a.rs"), ""));
        next(&mut seen, method("textDocument/didOpen")).await;
        let inject = injectors.lock().unwrap()[0].clone();
        inject
            .send(json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": 1, "value": {"kind": "begin", "title": "Indexing", "percentage": 40}}}))
            .unwrap();
        let busy = next(&mut events, |event| {
            matches!(
                event,
                LspEvent::Status {
                    status: ServerStatus::Busy(_),
                    ..
                }
            )
        })
        .await;
        assert_eq!(
            busy,
            LspEvent::Status {
                group: "rust-analyzer".into(),
                name: "rust-analyzer".into(),
                status: ServerStatus::Busy("Indexing 40%".into())
            }
        );
        inject
            .send(json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": 1, "value": {"kind": "end"}}}))
            .unwrap();
        next(&mut events, |event| {
            matches!(
                event,
                LspEvent::Status {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        handle.shutdown(Duration::from_secs(5)).await;

        let missing: Launch = Arc::new(|_, _| Err(io::Error::from(io::ErrorKind::NotFound)));
        let (handle, mut events) = spawn_with(missing);
        handle.send(open(Path::new("/w/a.rs"), ""));
        handle.send(open(Path::new("/w/b.rs"), ""));
        let notice = next(&mut events, |event| matches!(event, LspEvent::Notice(_))).await;
        assert_eq!(
            notice,
            LspEvent::Notice(
                "No language server found. Install rust-analyzer for code diagnostics".into()
            )
        );
        handle.shutdown(Duration::from_secs(5)).await;
        assert!(
            events.recv().await.is_none(),
            "the second file does not repeat the notice"
        );
    }

    #[tokio::test]
    async fn stop_shuts_a_server_down_and_the_next_open_starts_a_new_one() {
        let (launch, mut seen, injectors) = fake_launcher(json!({"textDocumentSync": 2}));
        let (handle, mut events) = spawn_with(launch);
        handle.send(open(Path::new("/w/a.rs"), ""));
        next(&mut seen, method("textDocument/didOpen")).await;
        handle.send(LspRequest::Stop {
            name: "rust-analyzer".into(),
        });
        next(&mut seen, method("shutdown")).await;
        let stopped = next_event(&mut events, |event| {
            matches!(event, LspEvent::Stopped { .. })
        })
        .await;
        assert_eq!(
            stopped,
            LspEvent::Stopped {
                group: "rust-analyzer".into(),
                name: "rust-analyzer".into(),
            }
        );
        // The App closes and reopens its files; a fresh server starts for them.
        handle.send(LspRequest::Close {
            path: PathBuf::from("/w/a.rs"),
        });
        handle.send(open(Path::new("/w/a.rs"), "fn main() {}\n"));
        next(&mut seen, method("initialize")).await;
        let opened = next(&mut seen, method("textDocument/didOpen")).await;
        assert_eq!(opened["params"]["textDocument"]["text"], "fn main() {}\n");
        assert_eq!(injectors.lock().unwrap().len(), 2, "a second process");
        handle.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn restart_looks_for_a_missing_program_again() {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_calls = calls.clone();
        let missing: Launch = Arc::new(move |spec, _| {
            seen_calls.lock().unwrap().push(spec.name.clone());
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        let (handle, mut events) = spawn_with(missing);
        handle.send(open(Path::new("/w/a.rs"), ""));
        next_event(&mut events, |event| matches!(event, LspEvent::Notice(_))).await;
        handle.send(open(Path::new("/w/b.rs"), ""));
        // Remembered as missing: the second file does not try again.
        handle.send(LspRequest::Restart {
            name: "rust-analyzer".into(),
        });
        handle.send(open(Path::new("/w/c.rs"), ""));
        handle.shutdown(Duration::from_secs(5)).await;
        while events.recv().await.is_some() {}
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["rust-analyzer", "rust-analyzer"],
            "tried for a.rs, skipped for b.rs, tried again after Restart"
        );
    }
}
