//! JSON-RPC 2.0 framing over a language server's stdio: a `Content-Length`
//! header block, a blank line, then exactly that many bytes of JSON.

use std::io;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A body larger than this is treated as a broken stream rather than allocated.
const MAX_BODY: usize = 256 * 1024 * 1024;

/// Reads one message body. `Ok(None)` is a clean end of stream.
pub async fn read_body<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid(format!("bad Content-Length: {}", value.trim())))?;
            length = Some(parsed);
        }
    }
    let length = length.unwrap_or(0);
    if length > MAX_BODY {
        return Err(invalid(format!("message of {length} bytes is too large")));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(body))
}

pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Value,
) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(|err| invalid(err.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn written_messages_read_back_in_order() {
        let (client, server) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            let mut client = client;
            write_message(&mut client, &json!({"id": 1, "text": "สวัสดี 🦆"}))
                .await
                .unwrap();
            write_message(&mut client, &json!({"id": 2})).await.unwrap();
        });
        let mut reader = BufReader::new(server);
        let first = read_body(&mut reader).await.unwrap().unwrap();
        let second = read_body(&mut reader).await.unwrap().unwrap();
        writer.await.unwrap();
        let first: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(first["text"], "สวัสดี 🦆");
        assert_eq!(serde_json::from_slice::<Value>(&second).unwrap()["id"], 2);
        assert!(read_body(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn extra_headers_and_header_case_are_accepted() {
        let raw = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
        let mut reader = BufReader::new(&raw[..]);
        assert_eq!(read_body(&mut reader).await.unwrap().unwrap(), b"{}");
    }

    #[tokio::test]
    async fn a_bad_length_is_an_error() {
        let raw = b"Content-Length: nope\r\n\r\n{}";
        let mut reader = BufReader::new(&raw[..]);
        assert!(read_body(&mut reader).await.is_err());
    }
}
