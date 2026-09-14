//! Standalone Papyrus LSP server.
//!
//! Speaks LSP over stdio. The IDE plugin (`.claude-plugin/plugin.json`)
//! registers this binary as the language server for `.psc` files.
//!
//! All parsing/resolving happens inside `papyrus_core`. This binary owns the
//! LSP protocol state (open documents, capabilities) and the `db_id` for the
//! workspace.

use papyrus_core::session;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct Backend {
    client: Client,
    /// LSP `Url` → session id from `papyrus_core::session`.
    docs: Mutex<HashMap<Url, u64>>,
    db_id: Mutex<Option<u64>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            docs: Mutex::new(HashMap::new()),
            db_id: Mutex::new(None),
        }
    }

    fn open_db_for_workspace(&self, root: Option<PathBuf>) {
        // Convention: `<root>/data/fo4_scripts.db` (matches `modkit index build`
        // output). Source dirs are `<root>/Scripts/Source/User`,
        // `<root>/Scripts/Source/Base` if they exist.
        let Some(root) = root else { return };
        let db_path = root.join("data").join("fo4_scripts.db");
        let db_path_str = db_path.to_string_lossy().to_string();

        let mut source_dirs = Vec::new();
        for sub in ["Scripts/Source/User", "Scripts/Source/Base"] {
            let p = root.join(sub);
            if p.is_dir() {
                source_dirs.push(p.to_string_lossy().to_string());
            }
        }

        let id = session::db_open(&db_path_str, &source_dirs);
        *self.db_id.lock().unwrap() = Some(id);
    }

    async fn publish_diagnostics(&self, uri: Url, sid: u64) {
        let diags_json = match papyrus_core_session_diagnostics(sid) {
            Some(s) => s,
            None => return,
        };
        let raw: Vec<RawDiagnostic> = match serde_json::from_str(&diags_json) {
            Ok(v) => v,
            Err(_) => return,
        };
        let diagnostics: Vec<Diagnostic> = raw.into_iter().map(Into::into).collect();
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

fn papyrus_core_session_diagnostics(sid: u64) -> Option<String> {
    let diags = session::session_diagnostics(sid)?;
    Some(serde_json::to_string(&diags).ok()?)
}

#[derive(serde::Deserialize)]
struct RawDiagnostic {
    line: u32,
    col: u32,
    end_line: u32,
    end_col: u32,
    message: String,
    severity: u8,
}

impl From<RawDiagnostic> for Diagnostic {
    fn from(d: RawDiagnostic) -> Self {
        let sev = match d.severity {
            1 => DiagnosticSeverity::ERROR,
            2 => DiagnosticSeverity::WARNING,
            3 => DiagnosticSeverity::INFORMATION,
            _ => DiagnosticSeverity::HINT,
        };
        Diagnostic {
            range: Range {
                start: Position {
                    line: d.line.saturating_sub(1),
                    character: d.col.saturating_sub(1),
                },
                end: Position {
                    line: d.end_line.saturating_sub(1),
                    character: d.end_col.saturating_sub(1),
                },
            },
            severity: Some(sev),
            message: d.message,
            source: Some("papyrus".into()),
            ..Default::default()
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Resolve workspace root from any of the standard slots.
        let root_path: Option<PathBuf> = params
            .workspace_folders
            .as_ref()
            .and_then(|f| f.first())
            .and_then(|f| f.uri.to_file_path().ok())
            .or_else(|| {
                #[allow(deprecated)]
                params.root_uri.as_ref().and_then(|u| u.to_file_path().ok())
            });
        self.open_db_for_workspace(root_path);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "papyrus-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "papyrus-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let db_id = *self.db_id.lock().unwrap();
        let sid = session::session_open(uri.to_string(), text, db_id);
        self.docs.lock().unwrap().insert(uri.clone(), sid);
        self.publish_diagnostics(uri, sid).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let sid = match self.docs.lock().unwrap().get(&uri) {
            Some(id) => *id,
            None => return,
        };
        // We declared FULL sync, so each change carries the full document.
        if let Some(change) = params.content_changes.into_iter().next() {
            session::session_replace_text(sid, change.text);
        }
        self.publish_diagnostics(uri, sid).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let sid = match self.docs.lock().unwrap().get(&uri) {
            Some(id) => *id,
            None => return,
        };
        self.publish_diagnostics(uri, sid).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(sid) = self.docs.lock().unwrap().remove(&uri) {
            session::session_close(sid);
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let sid = match self.docs.lock().unwrap().get(&uri) {
            Some(id) => *id,
            None => return Ok(None),
        };
        let Some(ast) = session::session_ast(sid) else {
            return Ok(None);
        };

        let mut symbols = Vec::new();
        // Properties
        for p in &ast.properties {
            symbols.push(symbol_info(
                &p.name,
                SymbolKind::PROPERTY,
                &p.pos,
                Some(&p.ty),
            ));
        }
        // Variables
        for v in &ast.variables {
            symbols.push(symbol_info(
                &v.name,
                SymbolKind::VARIABLE,
                &v.pos,
                Some(&v.ty),
            ));
        }
        // Functions
        for f in &ast.functions {
            symbols.push(symbol_info(
                &f.name,
                SymbolKind::FUNCTION,
                &f.pos,
                Some(&f.return_type),
            ));
        }
        // Events
        for e in &ast.events {
            symbols.push(symbol_info(&e.name, SymbolKind::EVENT, &e.pos, None));
        }
        // States (and their nested fns/events)
        for s in &ast.states {
            symbols.push(symbol_info(&s.name, SymbolKind::CLASS, &s.pos, None));
            for f in &s.functions {
                symbols.push(symbol_info(
                    &format!("{}::{}", s.name, f.name),
                    SymbolKind::FUNCTION,
                    &f.pos,
                    Some(&f.return_type),
                ));
            }
            for e in &s.events {
                symbols.push(symbol_info(
                    &format!("{}::{}", s.name, e.name),
                    SymbolKind::EVENT,
                    &e.pos,
                    None,
                ));
            }
        }

        Ok(Some(DocumentSymbolResponse::Flat(
            symbols
                .into_iter()
                .map(|si| SymbolInformation {
                    name: si.0,
                    kind: si.1,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: si.2,
                    },
                    container_name: si.3,
                })
                .collect(),
        )))
    }
}

fn symbol_info(
    name: &str,
    kind: SymbolKind,
    pos: &papyrus_core::ast::Pos,
    container: Option<&str>,
) -> (String, SymbolKind, Range, Option<String>) {
    let range = Range {
        start: Position {
            line: pos.line.saturating_sub(1),
            character: pos.col.saturating_sub(1),
        },
        end: Position {
            line: pos.end_line.saturating_sub(1),
            character: pos.end_col.saturating_sub(1),
        },
    };
    (
        name.to_owned(),
        kind,
        range,
        container.map(|s| s.to_owned()),
    )
}

#[allow(dead_code)]
fn _ensure_value_used(_: Value) {}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
