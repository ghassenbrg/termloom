//! Base protocol framing, shared by the LSP and DAP clients.
//!
//! Both protocols use the same envelope: `Content-Length: N\r\n\r\n` followed
//! by N bytes of JSON. The decoder is a pure state machine so it can be tested
//! against split reads, extra headers and malformed input.

use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// Serialise a message with its header.
pub fn encode(message: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(message).expect("serialising a JSON-RPC message");
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Write a message to a stream.
pub fn write_message(writer: &mut impl Write, message: &Value) -> Result<()> {
    writer.write_all(&encode(message))?;
    writer.flush()?;
    Ok(())
}

/// Read one message from a buffered stream. `Ok(None)` means clean EOF.
pub fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .context("parsing the Content-Length header")?,
                );
            }
            // Other headers (Content-Type) are accepted and ignored.
        } else {
            bail!("malformed header line: {trimmed:?}");
        }
    }

    let Some(length) = content_length else {
        bail!("message header is missing Content-Length");
    };
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .context("reading the message body")?;
    let value: Value = serde_json::from_slice(&body).context("parsing the message body")?;
    Ok(Some(value))
}

/// Incremental decoder for callers that receive arbitrary byte chunks.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Feed bytes and return every complete message they produced.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(header_end) = find_header_end(&self.buffer) {
            let header = String::from_utf8_lossy(&self.buffer[..header_end]).to_string();
            let Some(length) = parse_content_length(&header)? else {
                bail!("message header is missing Content-Length");
            };
            let total = header_end + 4 + length;
            if self.buffer.len() < total {
                break;
            }
            let body = &self.buffer[header_end + 4..total];
            let value: Value = serde_json::from_slice(body).context("parsing the message body")?;
            out.push(value);
            self.buffer.drain(..total);
        }
        Ok(out)
    }

    /// Bytes buffered but not yet forming a complete message.
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(header: &str) -> Result<Option<usize>> {
    for line in header.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return Ok(Some(
                    value
                        .trim()
                        .parse()
                        .context("parsing the Content-Length header")?,
                ));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodes_with_a_content_length_header() {
        let bytes = encode(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("Content-Length: "), "{text}");
        let (header, body) = text.split_once("\r\n\r\n").unwrap();
        let length: usize = header
            .trim_start_matches("Content-Length: ")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(length, body.len());
    }

    #[test]
    fn round_trips_a_message() {
        let message = json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}});
        let bytes = encode(&message);
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(message));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn reads_several_messages_from_one_stream() {
        let mut bytes = encode(&json!({"id":1}));
        bytes.extend(encode(&json!({"id":2})));
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap().unwrap()["id"], 1);
        assert_eq!(read_message(&mut reader).unwrap().unwrap()["id"], 2);
    }

    #[test]
    fn tolerates_extra_headers() {
        let body = br#"{"id":3}"#;
        let raw = format!(
            "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n",
            body.len()
        );
        let mut bytes = raw.into_bytes();
        bytes.extend_from_slice(body);
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap().unwrap()["id"], 3);
    }

    #[test]
    fn rejects_a_missing_content_length() {
        let mut bytes = b"Content-Type: text/plain\r\n\r\n".to_vec();
        bytes.extend_from_slice(b"{}");
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn decoder_handles_split_reads() {
        let mut decoder = Decoder::new();
        let bytes = encode(&json!({"method":"initialized"}));
        let (first, second) = bytes.split_at(7);
        assert!(decoder.feed(first).unwrap().is_empty());
        assert!(decoder.pending() > 0);
        let messages = decoder.feed(second).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "initialized");
        assert_eq!(decoder.pending(), 0);
    }

    #[test]
    fn decoder_handles_several_messages_in_one_chunk() {
        let mut decoder = Decoder::new();
        let mut bytes = encode(&json!({"id":1}));
        bytes.extend(encode(&json!({"id":2})));
        bytes.extend(encode(&json!({"id":3})));
        let messages = decoder.feed(&bytes).unwrap();
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn decoder_reports_invalid_json() {
        let mut decoder = Decoder::new();
        let mut bytes = b"Content-Length: 3\r\n\r\n".to_vec();
        bytes.extend_from_slice(b"{ x");
        assert!(decoder.feed(&bytes).is_err());
    }

    #[test]
    fn decoder_handles_utf8_bodies() {
        let mut decoder = Decoder::new();
        let message = json!({"text":"héllo → wörld"});
        let messages = decoder.feed(&encode(&message)).unwrap();
        assert_eq!(messages[0], message);
    }
}
