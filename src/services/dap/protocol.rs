//! Debug Adapter Protocol payloads and conversions.
//!
//! DAP uses the same `Content-Length` envelope as LSP but a different message
//! shape (`request`/`response`/`event` with sequence numbers).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::debug::{DebugCapabilities, DebugThread, StackFrame, Variable};

/// Build a request message.
pub fn request(seq: i64, command: &str, arguments: Value) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments
    })
}

/// `initialize` arguments describing what TermLoom supports.
pub fn initialize_arguments() -> Value {
    json!({
        "clientID": "termloom",
        "clientName": crate::PRODUCT_NAME,
        "adapterID": "termloom",
        "locale": "en",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        "supportsVariablePaging": false,
        "supportsRunInTerminalRequest": false,
        "supportsMemoryReferences": false,
        "supportsProgressReporting": false,
        "supportsInvalidatedEvent": false
    })
}

/// `setBreakpoints` arguments for one source file.
///
/// `lines` are 0-based editor lines; DAP wants 1-based.
pub fn set_breakpoints(path: &Path, lines: &[usize]) -> Value {
    json!({
        "source": {
            "path": path.to_string_lossy(),
            "name": path.file_name().map(|n| n.to_string_lossy().to_string())
        },
        "breakpoints": lines
            .iter()
            .map(|line| json!({ "line": line + 1 }))
            .collect::<Vec<_>>(),
        "sourceModified": false
    })
}

pub fn stack_trace(thread_id: i64) -> Value {
    json!({ "threadId": thread_id, "startFrame": 0, "levels": 50 })
}

pub fn scopes(frame_id: i64) -> Value {
    json!({ "frameId": frame_id })
}

pub fn variables(reference: i64) -> Value {
    json!({ "variablesReference": reference })
}

pub fn evaluate(expression: &str, frame_id: Option<i64>) -> Value {
    match frame_id {
        Some(id) => json!({ "expression": expression, "frameId": id, "context": "repl" }),
        None => json!({ "expression": expression, "context": "repl" }),
    }
}

/// Parse the `capabilities` body of an `initialize` response.
pub fn parse_capabilities(body: &Value) -> DebugCapabilities {
    let flag = |key: &str| body[key].as_bool().unwrap_or(false);
    DebugCapabilities {
        configuration_done: flag("supportsConfigurationDoneRequest"),
        terminate: flag("supportsTerminateRequest"),
        restart: flag("supportsRestartRequest"),
        step_back: flag("supportsStepBack"),
        evaluate_for_hovers: flag("supportsEvaluateForHovers"),
        conditional_breakpoints: flag("supportsConditionalBreakpoints"),
        set_variable: flag("supportsSetVariable"),
    }
}

/// Parse a `stackTrace` response body.
pub fn parse_stack_frames(body: &Value) -> Vec<StackFrame> {
    body["stackFrames"]
        .as_array()
        .map(|frames| {
            frames
                .iter()
                .map(|frame| StackFrame {
                    id: frame["id"].as_i64().unwrap_or(0),
                    name: frame["name"].as_str().unwrap_or("<unknown>").to_string(),
                    path: frame["source"]["path"].as_str().map(PathBuf::from),
                    line: frame["line"].as_u64().unwrap_or(0) as usize,
                    column: frame["column"].as_u64().unwrap_or(0) as usize,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a `threads` response body.
pub fn parse_threads(body: &Value) -> Vec<DebugThread> {
    body["threads"]
        .as_array()
        .map(|threads| {
            threads
                .iter()
                .map(|thread| DebugThread {
                    id: thread["id"].as_i64().unwrap_or(0),
                    name: thread["name"].as_str().unwrap_or("thread").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a `variables` response body at a given nesting depth.
pub fn parse_variables(body: &Value, depth: usize) -> Vec<Variable> {
    body["variables"]
        .as_array()
        .map(|variables| {
            variables
                .iter()
                .map(|variable| Variable {
                    name: variable["name"].as_str().unwrap_or_default().to_string(),
                    value: variable["value"]
                        .as_str()
                        .unwrap_or_default()
                        .replace('\n', " "),
                    type_name: variable["type"].as_str().map(str::to_string),
                    variables_reference: variable["variablesReference"].as_i64().unwrap_or(0),
                    depth,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Scope references from a `scopes` response, named for display.
pub fn parse_scopes(body: &Value) -> Vec<(String, i64)> {
    body["scopes"]
        .as_array()
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|scope| {
                    let reference = scope["variablesReference"].as_i64()?;
                    (reference != 0).then(|| {
                        (
                            scope["name"].as_str().unwrap_or("scope").to_string(),
                            reference,
                        )
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Verified breakpoints from a `setBreakpoints` response (0-based lines).
pub fn parse_breakpoints(body: &Value) -> Vec<(usize, bool)> {
    body["breakpoints"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item["line"].as_u64().unwrap_or(1).saturating_sub(1) as usize,
                        item["verified"].as_bool().unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoint_lines_become_one_based() {
        let params = set_breakpoints(Path::new("/repo/src/main.rs"), &[0, 41]);
        assert_eq!(params["breakpoints"][0]["line"], 1);
        assert_eq!(params["breakpoints"][1]["line"], 42);
        assert_eq!(params["source"]["name"], "main.rs");
    }

    #[test]
    fn breakpoint_responses_become_zero_based() {
        let body = json!({"breakpoints":[{"line":42,"verified":true},{"line":7,"verified":false}]});
        assert_eq!(parse_breakpoints(&body), vec![(41, true), (6, false)]);
    }

    #[test]
    fn capabilities_default_to_unsupported() {
        let capabilities = parse_capabilities(&json!({"supportsTerminateRequest": true}));
        assert!(capabilities.terminate);
        assert!(!capabilities.restart);
        assert!(!capabilities.step_back);
    }

    #[test]
    fn stack_frames_carry_their_source() {
        let body = json!({"stackFrames":[{
            "id": 1000,
            "name": "main",
            "line": 12,
            "column": 5,
            "source": {"path": "/repo/src/main.rs"}
        }]});
        let frames = parse_stack_frames(&body);
        assert_eq!(frames[0].name, "main");
        assert_eq!(frames[0].path, Some(PathBuf::from("/repo/src/main.rs")));
        assert_eq!(frames[0].line, 12);
    }

    #[test]
    fn variables_flatten_with_a_depth() {
        let body = json!({"variables":[
            {"name":"count","value":"3","type":"i32","variablesReference":0},
            {"name":"list","value":"Vec","variablesReference":42}
        ]});
        let variables = parse_variables(&body, 1);
        assert_eq!(variables[0].name, "count");
        assert_eq!(variables[0].depth, 1);
        assert_eq!(variables[1].variables_reference, 42);
    }

    #[test]
    fn scopes_skip_empty_references() {
        let body = json!({"scopes":[
            {"name":"Locals","variablesReference":5},
            {"name":"Registers","variablesReference":0}
        ]});
        assert_eq!(parse_scopes(&body), vec![("Locals".to_string(), 5)]);
    }

    #[test]
    fn threads_are_parsed() {
        let body = json!({"threads":[{"id":1,"name":"main"}]});
        assert_eq!(parse_threads(&body)[0].name, "main");
    }
}
