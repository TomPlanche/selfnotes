//! A stdio language server that completes and describes `@mention`s from the people roster.
//!
//! The protocol surface is deliberately small: enough of the handshake and document sync to know what the buffer
//! contains, then `textDocument/completion` after an `@` and `textDocument/hover` over one. Editors talk to it over
//! stdin and stdout with the usual `Content-Length` framing, so `selfnotes lsp` is what an editor extension launches,
//! never something you run by hand.
//!
//! The roster is re-read whenever `people.toml`'s modification time changes, so adding a colleague takes effect on the
//! next keystroke rather than on the next restart.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::config;
use crate::people::{self, Directory};

/// JSON-RPC error code for a method the server does not implement.
const METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC error code for a request that failed while being handled.
const INTERNAL_ERROR: i64 = -32603;

/// `CompletionItemKind::Value`: the closest thing the protocol has to "a person".
const COMPLETION_KIND: i64 = 12;

/// `MessageType::Info`, used for the startup line written to the editor's language-server log.
const LOG_INFO: i64 = 3;

/// Most completions returned for one `@` prefix, so a large roster cannot flood the popup.
const MAX_COMPLETIONS: usize = 100;

/// Run the language server until the client asks it to exit or closes stdin.
pub fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut server = Server::default();

    while let Some(message) = read_message(&mut input)? {
        // A message without a method is a response to one of our own notifications; there is nothing to answer.
        let Some(method) = message.get("method").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };

        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match server.handle(&method, &params, &mut output) {
            Ok(Outcome::Exit) => break,
            Ok(Outcome::Reply(result)) => {
                if let Some(id) = id {
                    write_message(&mut output, &json!({ "jsonrpc": "2.0", "id": id, "result": result }))?;
                }
            },
            // Every method the server handles as a request replies with `Outcome::Reply`, so a quiet outcome that
            // still carries an id can only be a method this server does not implement.
            Ok(Outcome::Quiet) => {
                if let Some(id) = id {
                    let message = format!("unknown method `{method}`");

                    write_message(&mut output, &error_response(&id, METHOD_NOT_FOUND, &message))?;
                }
            },
            Err(err) => match id {
                Some(id) => write_message(&mut output, &error_response(&id, INTERNAL_ERROR, &format!("{err:#}")))?,
                None => log(&mut output, &format!("selfnotes: {method} failed: {err:#}"))?,
            },
        }
    }

    Ok(())
}

/// What the dispatcher owes the client after a message.
enum Outcome {
    /// Reply to a request with this result.
    Reply(Value),
    /// Nothing to send back.
    Quiet,
    /// The client asked the server to exit.
    Exit,
}

/// How the client counts the `character` field of a position.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    /// UTF-16 code units, the protocol's default and the only encoding every client supports.
    #[default]
    Utf16,
    /// Bytes, negotiated when the client offers `utf-8`.
    Utf8,
}

impl Encoding {
    /// The name announced back to the client in the server's capabilities.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Utf16 => "utf-16",
            Self::Utf8 => "utf-8",
        }
    }
}

/// A resolved cursor: the line it sits on, that line's number, and its byte offset within the line.
struct Cursor<'a> {
    line: &'a str,
    number: usize,
    col: usize,
}

#[derive(Default)]
struct Server {
    /// Open documents by URI, held in full because document sync is full-text.
    documents: HashMap<String, String>,
    directory: Directory,
    /// Roster file being watched, once the config has resolved one.
    people_path: Option<PathBuf>,
    /// Modification time of the roster as it was last read, so edits are picked up without a restart.
    people_stamp: Option<SystemTime>,
    /// Whether the roster has been read at least once.
    loaded: bool,
    encoding: Encoding,
    /// Whether the client understands an `InsertReplaceEdit` on a completion item.
    insert_replace: bool,
}

impl Server {
    fn handle(&mut self, method: &str, params: &Value, output: &mut impl Write) -> Result<Outcome> {
        match method {
            "initialize" => Ok(Outcome::Reply(self.initialize(params, output)?)),
            "shutdown" => Ok(Outcome::Reply(Value::Null)),
            "exit" => Ok(Outcome::Exit),
            "textDocument/completion" => Ok(Outcome::Reply(self.complete(params))),
            "textDocument/hover" => Ok(Outcome::Reply(self.hover(params))),
            "textDocument/didOpen" => {
                if let Some(uri) = string_at(params, "/textDocument/uri")
                    && let Some(text) = string_at(params, "/textDocument/text")
                {
                    self.documents.insert(uri.to_owned(), text.to_owned());
                }

                Ok(Outcome::Quiet)
            },
            "textDocument/didChange" => {
                // Sync is full, so the last change of the batch carries the whole document.
                if let Some(uri) = string_at(params, "/textDocument/uri")
                    && let Some(text) = params
                        .pointer("/contentChanges")
                        .and_then(Value::as_array)
                        .and_then(|changes| changes.last())
                        .and_then(|change| change.get("text"))
                        .and_then(Value::as_str)
                {
                    self.documents.insert(uri.to_owned(), text.to_owned());
                }

                Ok(Outcome::Quiet)
            },
            "textDocument/didClose" => {
                if let Some(uri) = string_at(params, "/textDocument/uri") {
                    self.documents.remove(uri);
                }

                Ok(Outcome::Quiet)
            },
            // Notifications with nothing to do, and anything else this server does not implement.
            _ => Ok(Outcome::Quiet),
        }
    }

    /// Answer the handshake: agree on a position encoding, locate the roster, and announce what the server can do.
    fn initialize(&mut self, params: &Value, output: &mut impl Write) -> Result<Value> {
        self.encoding = negotiate_encoding(params);
        self.insert_replace = params
            .pointer("/capabilities/textDocument/completion/completionItem/insertReplaceSupport")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Which config layers apply, and so which roster, is resolved from the working directory, and an editor is
        // free to start its language servers anywhere. Move to the workspace root so a path-scoped `[[overrides]]`
        // entry selects the right roster for the notes being edited.
        if let Some(root) = workspace_root(params) {
            std::env::set_current_dir(&root).with_context(|| format!("entering workspace {}", root.display()))?;
        }

        let config = config::load().unwrap_or_default();
        self.people_path = people::path(&config);
        self.reload_people();

        let source = self
            .people_path
            .as_ref()
            .map_or_else(|| "<no people file>".to_owned(), |path| path.display().to_string());
        log(
            output,
            &format!("selfnotes: {} people loaded from {source}", self.directory.people.len()),
        )?;

        Ok(json!({
            "capabilities": {
                "positionEncoding": self.encoding.as_str(),
                // 1 is `TextDocumentSyncKind::Full`.
                "textDocumentSync": { "openClose": true, "change": 1 },
                "completionProvider": { "triggerCharacters": ["@"], "resolveProvider": false },
                "hoverProvider": true,
            },
            "serverInfo": { "name": "selfnotes", "version": env!("CARGO_PKG_VERSION") },
        }))
    }

    /// Re-read the roster when its modification time has moved since the last read.
    ///
    /// A file that fails to parse leaves the last good roster in place, so a half-typed edit does not empty the
    /// completions while you are in the middle of writing a note.
    fn reload_people(&mut self) {
        let Some(path) = self.people_path.clone() else {
            return;
        };

        let stamp = std::fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        if self.loaded && stamp == self.people_stamp {
            return;
        }

        if let Ok(directory) = people::read(&path) {
            self.directory = directory;
        }

        self.people_stamp = stamp;
        self.loaded = true;
    }

    /// Completions for the `@mention` being typed at the cursor.
    fn complete(&mut self, params: &Value) -> Value {
        self.reload_people();

        let Some(cursor) = self.cursor(params) else {
            return empty_completions();
        };

        let Some(mention) = people::mention_before(cursor.line, cursor.col) else {
            return empty_completions();
        };

        // What was typed is replaced whatever the client supports; a client that understands an `InsertReplaceEdit`
        // also gets a replace range covering the whole mention, so accepting a completion with the cursor inside one
        // rewrites it rather than leaving its tail behind.
        let typed = self.range(&cursor, mention.start, mention.end);
        let edit = if self.insert_replace {
            let whole = people::mention_at(cursor.line, cursor.col).unwrap_or(mention);

            json!({ "insert": typed, "replace": self.range(&cursor, whole.start, whole.end) })
        } else {
            json!({ "range": typed })
        };

        let matches = self.directory.matches(mention.text);
        let items: Vec<Value> = matches
            .iter()
            .take(MAX_COMPLETIONS)
            .enumerate()
            .map(|(rank, hit)| {
                let text = format!("@{}", hit.person.handle);
                let mut text_edit = edit.clone();
                text_edit["newText"] = Value::String(text.clone());

                json!({
                    "label": text,
                    "kind": COMPLETION_KIND,
                    "detail": hit.person.detail(),
                    "documentation": { "kind": "markdown", "value": hit.person.describe() },
                    "filterText": hit.person.filter_text(),
                    // `matches` is already ordered; the rank freezes that order against the client's own sorting.
                    "sortText": format!("{rank:04}"),
                    "textEdit": text_edit,
                })
            })
            .collect();

        json!({ "isIncomplete": matches.len() > items.len(), "items": items })
    }

    /// The roster entry for the `@mention` under the cursor, as a hover popup.
    fn hover(&mut self, params: &Value) -> Value {
        self.reload_people();

        let Some(cursor) = self.cursor(params) else {
            return Value::Null;
        };

        let Some(mention) = people::mention_at(cursor.line, cursor.col) else {
            return Value::Null;
        };

        let Some(person) = self.directory.resolve(mention.text) else {
            return Value::Null;
        };

        json!({
            "contents": { "kind": "markdown", "value": person.describe() },
            "range": self.range(&cursor, mention.start, mention.end),
        })
    }

    /// Resolve a request's `textDocument`/`position` pair against the open documents.
    fn cursor(&self, params: &Value) -> Option<Cursor<'_>> {
        let uri = string_at(params, "/textDocument/uri")?;
        let text = self.documents.get(uri)?;
        let number = index_at(params, "/position/line")?;
        let character = index_at(params, "/position/character")?;
        let line = line_at(text, number)?;

        Some(Cursor {
            line,
            number,
            col: byte_col(line, character, self.encoding),
        })
    }

    /// An LSP range covering the byte offsets `start..end` of the cursor's line.
    fn range(&self, cursor: &Cursor<'_>, start: usize, end: usize) -> Value {
        json!({
            "start": { "line": cursor.number, "character": lsp_col(cursor.line, start, self.encoding) },
            "end": { "line": cursor.number, "character": lsp_col(cursor.line, end, self.encoding) },
        })
    }
}

/// A completion response with nothing to offer.
fn empty_completions() -> Value {
    json!({ "isIncomplete": false, "items": [] })
}

/// Read one LSP message, or `None` once the client closes stdin.
fn read_message(input: &mut impl BufRead) -> Result<Option<Value>> {
    let mut length = None;

    loop {
        let mut header = String::new();
        if input.read_line(&mut header)? == 0 {
            return Ok(None);
        }

        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            let value = value.trim();
            length = Some(
                value
                    .parse()
                    .with_context(|| format!("invalid Content-Length `{value}`"))?,
            );
        }
    }

    let length: usize = length.context("a message header carried no Content-Length")?;
    let mut body = vec![0; length];
    input.read_exact(&mut body).context("reading a message body")?;

    serde_json::from_slice(&body)
        .context("parsing a message body")
        .map(Some)
}

/// Frame and flush one message to the client.
fn write_message(output: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message).context("serializing a message")?;

    write!(output, "Content-Length: {}\r\n\r\n", body.len()).context("writing a message header")?;
    output.write_all(&body).context("writing a message body")?;

    output.flush().context("flushing a message")
}

/// Send a `window/logMessage` notification, which lands in the editor's language-server log.
fn log(output: &mut impl Write, message: &str) -> Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "window/logMessage",
            "params": { "type": LOG_INFO, "message": message },
        }),
    )
}

/// A JSON-RPC error reply.
fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A string at `pointer` within `params`.
fn string_at<'a>(params: &'a Value, pointer: &str) -> Option<&'a str> {
    params.pointer(pointer).and_then(Value::as_str)
}

/// A non-negative integer at `pointer` within `params`.
fn index_at(params: &Value, pointer: &str) -> Option<usize> {
    usize::try_from(params.pointer(pointer)?.as_u64()?).ok()
}

/// Pick a position encoding the client understands, defaulting to the protocol's UTF-16.
fn negotiate_encoding(params: &Value) -> Encoding {
    let offers_utf8 = params
        .pointer("/capabilities/general/positionEncodings")
        .and_then(Value::as_array)
        .is_some_and(|offered| offered.iter().any(|encoding| encoding.as_str() == Some("utf-8")));

    if offers_utf8 { Encoding::Utf8 } else { Encoding::Utf16 }
}

/// The workspace directory the client opened, from `workspaceFolders` or the older `rootUri`/`rootPath`.
fn workspace_root(params: &Value) -> Option<PathBuf> {
    if let Some(uri) = string_at(params, "/workspaceFolders/0/uri").or_else(|| string_at(params, "/rootUri")) {
        return uri_to_path(uri);
    }

    string_at(params, "/rootPath").map(PathBuf::from)
}

/// Decode a `file://` URI into a path.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://")
        .map(|path| PathBuf::from(percent_decode(path)))
}

/// Undo the percent-escapes of a URI. Escapes are decoded as bytes, so a multi-byte character survives.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(escape) = input.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(escape, 16)
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// The `line`-th line of `text`, without its terminator.
fn line_at(text: &str, line: usize) -> Option<&str> {
    text.split('\n')
        .nth(line)
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

/// Byte offset within `line` of the position `character`, counted in `encoding`'s units.
fn byte_col(line: &str, character: usize, encoding: Encoding) -> usize {
    match encoding {
        // Already a byte offset, but a client may still point past the end of the line.
        Encoding::Utf8 => {
            let mut col = character.min(line.len());
            while !line.is_char_boundary(col) {
                col -= 1;
            }

            col
        },
        Encoding::Utf16 => {
            let mut units = 0;

            for (offset, character_at) in line.char_indices() {
                if units >= character {
                    return offset;
                }

                units += character_at.len_utf16();
            }

            line.len()
        },
    }
}

/// Position, in `encoding`'s units, of the byte offset `col` within `line`.
fn lsp_col(line: &str, col: usize, encoding: Encoding) -> usize {
    match encoding {
        Encoding::Utf8 => col,
        Encoding::Utf16 => line.get(..col).unwrap_or(line).chars().map(char::len_utf16).sum(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor as ByteCursor;

    use super::*;

    #[test]
    fn read_message_parses_framed_json() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#;
        let raw = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let mut input = ByteCursor::new(raw.into_bytes());

        let message = read_message(&mut input).unwrap().unwrap();
        assert_eq!(message["method"], "shutdown");

        // A second read hits the end of the stream.
        assert!(read_message(&mut input).unwrap().is_none());
    }

    #[test]
    fn read_message_tolerates_extra_headers() {
        let body = r#"{"method":"exit"}"#;
        let raw = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );

        let message = read_message(&mut ByteCursor::new(raw.into_bytes())).unwrap().unwrap();
        assert_eq!(message["method"], "exit");
    }

    #[test]
    fn write_message_frames_the_body() {
        let mut output = Vec::new();
        write_message(&mut output, &json!({ "ok": true })).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Content-Length: 11\r\n\r\n{\"ok\":true}"
        );
    }

    #[test]
    fn line_at_splits_on_both_line_endings() {
        assert_eq!(line_at("a\nb\nc", 1), Some("b"));
        assert_eq!(line_at("a\r\nb\r\n", 1), Some("b"));
        assert_eq!(line_at("a\nb", 5), None);
    }

    #[test]
    fn utf16_positions_round_trip_through_byte_offsets() {
        // "é" is two bytes but one UTF-16 unit; "😀" is four bytes and two units.
        let line = "é😀x";

        assert_eq!(byte_col(line, 0, Encoding::Utf16), 0);
        assert_eq!(byte_col(line, 1, Encoding::Utf16), 2);
        assert_eq!(byte_col(line, 3, Encoding::Utf16), 6);
        assert_eq!(lsp_col(line, 2, Encoding::Utf16), 1);
        assert_eq!(lsp_col(line, 6, Encoding::Utf16), 3);
    }

    #[test]
    fn byte_positions_are_clamped_to_character_boundaries() {
        let line = "é";

        assert_eq!(byte_col(line, 1, Encoding::Utf8), 0);
        assert_eq!(byte_col(line, 99, Encoding::Utf8), 2);
    }

    #[test]
    fn negotiate_encoding_prefers_utf8_when_offered() {
        let offered = json!({ "capabilities": { "general": { "positionEncodings": ["utf-8", "utf-16"] } } });
        assert_eq!(negotiate_encoding(&offered), Encoding::Utf8);

        assert_eq!(negotiate_encoding(&json!({})), Encoding::Utf16);
    }

    #[test]
    fn workspace_root_reads_folders_then_root_uri() {
        let params = json!({
            "workspaceFolders": [{ "uri": "file:///Users/me/my%20notes" }],
            "rootUri": "file:///elsewhere",
        });
        assert_eq!(workspace_root(&params), Some(PathBuf::from("/Users/me/my notes")));

        let older = json!({ "rootUri": "file:///Users/me/notes" });
        assert_eq!(workspace_root(&older), Some(PathBuf::from("/Users/me/notes")));
    }

    /// A server holding one open document, used to exercise the request handlers.
    fn server_with(text: &str, people: Vec<people::Person>) -> Server {
        let mut server = Server {
            directory: Directory { people },
            loaded: true,
            ..Server::default()
        };
        server.documents.insert("file:///notes.md".into(), text.to_owned());

        server
    }

    fn roster() -> Vec<people::Person> {
        vec![
            people::Person {
                handle: "jdoe".into(),
                name: Some("Jane Doe".into()),
                team: Some("backend".into()),
                ..people::Person::default()
            },
            people::Person {
                handle: "jsmith".into(),
                name: Some("John Smith".into()),
                ..people::Person::default()
            },
        ]
    }

    fn position(line: usize, character: usize) -> Value {
        json!({
            "textDocument": { "uri": "file:///notes.md" },
            "position": { "line": line, "character": character },
        })
    }

    #[test]
    fn completion_replaces_the_whole_mention() {
        let mut server = server_with("- [ ] ask @jd", roster());
        let response = server.complete(&position(0, 13));

        let items = response["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["label"], "@jdoe");
        assert_eq!(items[0]["detail"], "Jane Doe (backend)");
        assert_eq!(items[0]["textEdit"]["newText"], "@jdoe");
        // The edit covers `@jd`, so accepting it never leaves a doubled `@`.
        assert_eq!(items[0]["textEdit"]["range"]["start"]["character"], 10);
        assert_eq!(items[0]["textEdit"]["range"]["end"]["character"], 13);
    }

    #[test]
    fn completion_replaces_the_whole_mention_when_the_client_supports_it() {
        let mut server = server_with("ask @jdo about it", roster());
        server.insert_replace = true;

        // The cursor sits between `jd` and `o`.
        let response = server.complete(&position(0, 7));
        let edit = &response["items"][0]["textEdit"];

        // Only `@jd` was typed, but the replace range swallows the trailing `o` too.
        assert_eq!(edit["insert"]["end"]["character"], 7);
        assert_eq!(edit["replace"]["end"]["character"], 8);
        assert_eq!(edit["newText"], "@jdoe");
    }

    #[test]
    fn completion_offers_everyone_right_after_the_at() {
        let mut server = server_with("ping @", roster());
        let response = server.complete(&position(0, 6));

        assert_eq!(response["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn completion_stays_out_of_email_addresses() {
        let mut server = server_with("write to jane@ex", roster());
        let response = server.complete(&position(0, 16));

        assert!(response["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn hover_describes_the_mention_under_the_cursor() {
        let mut server = server_with("ping @jdoe today", roster());
        let response = server.hover(&position(0, 8));

        let text = response["contents"]["value"].as_str().unwrap();
        assert!(text.contains("Jane Doe"), "{text}");
        assert!(text.contains("backend"), "{text}");
        assert_eq!(response["range"]["start"]["character"], 5);
        assert_eq!(response["range"]["end"]["character"], 10);
    }

    #[test]
    fn hover_says_nothing_about_an_unknown_handle() {
        let mut server = server_with("ping @nobody", roster());

        assert_eq!(server.hover(&position(0, 8)), Value::Null);
    }

    #[test]
    fn document_sync_tracks_the_latest_text() {
        let mut server = server_with("", roster());
        let mut output = Vec::new();

        let change = json!({
            "textDocument": { "uri": "file:///notes.md" },
            "contentChanges": [{ "text": "ping @j" }],
        });
        server.handle("textDocument/didChange", &change, &mut output).unwrap();

        let response = server.complete(&position(0, 7));
        assert_eq!(response["items"].as_array().unwrap().len(), 2);

        let close = json!({ "textDocument": { "uri": "file:///notes.md" } });
        server.handle("textDocument/didClose", &close, &mut output).unwrap();
        assert!(server.documents.is_empty());
    }
}
