//! `file://` URIs for local paths. Windows drive paths become `file:///C:/...`
//! and UNC shares become `file://server/share/...`.

use std::path::{Path, PathBuf};

pub fn from_path(path: &Path) -> String {
    encode(&path.to_string_lossy(), cfg!(windows))
}

/// `None` for a URI that is not a local `file:` path.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    decode(uri, cfg!(windows)).map(PathBuf::from)
}

fn encode(path: &str, windows: bool) -> String {
    let mut uri = String::from("file://");
    if !windows {
        percent_encode(path, &mut uri);
        return uri;
    }
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| path.strip_prefix(r"\\?\").unwrap_or(path).to_string());
    let path = path.replace('\\', "/");
    if let Some(share) = path.strip_prefix("//") {
        let (host, rest) = share.split_once('/').unwrap_or((share, ""));
        uri.push_str(host);
        uri.push('/');
        percent_encode(rest, &mut uri);
    } else {
        if !path.starts_with('/') {
            uri.push('/');
        }
        percent_encode(&path, &mut uri);
    }
    uri
}

fn decode(uri: &str, windows: bool) -> Option<String> {
    let scheme = uri.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = &uri[7..];
    let (host, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, ""),
    };
    let path = percent_decode(path)?;
    let host = (!host.eq_ignore_ascii_case("localhost"))
        .then_some(host)
        .filter(|h| !h.is_empty());
    if !windows {
        return match host {
            Some(_) => None,
            None => Some(path),
        };
    }
    if let Some(host) = host {
        return Some(format!(r"\\{host}{}", path.replace('/', "\\")));
    }
    let bytes = path.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    let path = if drive {
        let mut out = path[1..2].to_ascii_uppercase();
        out.push_str(&path[2..]);
        out
    } else {
        path
    };
    Some(path.replace('/', "\\"))
}

fn percent_encode(text: &str, out: &mut String) {
    for byte in text.bytes() {
        let keep =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':');
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_drive_paths_round_trip() {
        let uri = encode(r"E:\Work\duck code\main.rs", true);
        assert_eq!(uri, "file:///E:/Work/duck%20code/main.rs");
        assert_eq!(decode(&uri, true).unwrap(), r"E:\Work\duck code\main.rs");
    }

    #[test]
    fn verbatim_prefix_is_dropped() {
        assert_eq!(encode(r"\\?\C:\a.rs", true), "file:///C:/a.rs");
    }

    #[test]
    fn server_spellings_of_a_drive_decode_to_the_same_path() {
        assert_eq!(
            decode("file:///e%3A/src/a.rs", true).unwrap(),
            r"E:\src\a.rs"
        );
        assert_eq!(decode("file:///e:/src/a.rs", true).unwrap(), r"E:\src\a.rs");
    }

    #[test]
    fn unc_shares_keep_their_host() {
        let uri = encode(r"\\server\share\a.rs", true);
        assert_eq!(uri, "file://server/share/a.rs");
        assert_eq!(decode(&uri, true).unwrap(), r"\\server\share\a.rs");
    }

    #[test]
    fn unix_paths_round_trip_with_non_ascii_names() {
        let uri = encode("/home/me/ไฟล์.rs", false);
        assert_eq!(
            uri,
            "file:///home/me/%E0%B9%84%E0%B8%9F%E0%B8%A5%E0%B9%8C.rs"
        );
        assert_eq!(decode(&uri, false).unwrap(), "/home/me/ไฟล์.rs");
    }

    #[test]
    fn other_schemes_are_not_paths() {
        assert!(decode("untitled:Untitled-1", true).is_none());
        assert!(decode("https://example.com/a.rs", false).is_none());
    }
}
