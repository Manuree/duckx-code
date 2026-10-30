//! Decode and encode buffer text at the file boundary.
//! The rope stays Unicode. A BOM selects UTF-8 or UTF-16 on open.
//! Windows-1252 and Windows-874 are used only when a command names them.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use super::fs::write_bytes;

/// Encoding of the bytes on disk. The buffer itself is always Unicode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
    Windows874,
}

impl TextEncoding {
    /// Names accepted by `encoding` and `reload`. Matching ignores case and `-`.
    pub fn parse(name: &str) -> Option<Self> {
        let folded: String = name
            .chars()
            .filter(|ch| !ch.is_whitespace() && *ch != '-' && *ch != '_')
            .flat_map(|ch| ch.to_lowercase())
            .collect();
        match folded.as_str() {
            "utf8" => Some(Self::Utf8),
            "utf8bom" => Some(Self::Utf8Bom),
            "utf16le" | "utf16" => Some(Self::Utf16Le),
            "utf16be" => Some(Self::Utf16Be),
            "windows1252" | "cp1252" => Some(Self::Windows1252),
            "windows874" | "cp874" | "tis620" => Some(Self::Windows874),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Windows1252 => "Windows-1252",
            Self::Windows874 => "Windows-874",
        }
    }

    /// Shown when a command names an encoding this editor does not know.
    pub fn names() -> &'static str {
        "utf-8, utf-8-bom, utf-16le, utf-16be, windows-1252, windows-874"
    }
}

/// Modified time and length captured when the editor last read or wrote the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: SystemTime,
    pub len: u64,
}

/// Text produced by a successful read, plus the encoding to use on the next save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: TextEncoding,
    pub stamp: FileStamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The path does not exist yet.
    Missing,
    /// The path exists and the bytes could not be read or decoded as requested.
    Failed(String),
    /// The bytes are not UTF-8 and have no UTF-8 or UTF-16 BOM.
    Undecoded(String),
}

impl OpenError {
    pub fn message(self, path: &Path) -> String {
        match self {
            Self::Missing => format!("Cannot open {}: not found", path.display()),
            Self::Failed(message) | Self::Undecoded(message) => message,
        }
    }
}

/// Reads `path`, detecting UTF-8 or a UTF-8 / UTF-16 BOM.
pub fn read_auto(path: &Path) -> Result<Decoded, OpenError> {
    let bytes = read_bytes(path)?;
    let encoding = detect(&bytes).ok_or_else(|| undecoded(path))?;
    let text = decode(&bytes, encoding).map_err(|_| undecoded(path))?;
    finish_read(path, text, encoding)
}

/// Reads `path` as `encoding`. A matching BOM is not kept in the text.
pub fn read_with(path: &Path, encoding: TextEncoding) -> Result<Decoded, OpenError> {
    let bytes = read_bytes(path)?;
    let text = decode(&bytes, encoding)
        .map_err(|detail| OpenError::Failed(format!("Cannot open {}: {detail}", path.display())))?;
    finish_read(path, text, encoding)
}

/// Encodes `text` and replaces `path` atomically. The returned stamp is the file after that write.
pub fn write_encoded(path: &Path, text: &str, encoding: TextEncoding) -> Result<FileStamp, String> {
    let bytes = encode(text, encoding)
        .map_err(|detail| format!("Cannot save {}: {detail}", path.display()))?;
    write_bytes(path, &bytes)?;
    file_stamp(path)
}

pub fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    let meta =
        fs::metadata(path).map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
    let modified = meta
        .modified()
        .map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
    Ok(FileStamp {
        modified,
        len: meta.len(),
    })
}

fn finish_read(path: &Path, text: String, encoding: TextEncoding) -> Result<Decoded, OpenError> {
    let stamp = file_stamp(path).map_err(OpenError::Failed)?;
    Ok(Decoded {
        text,
        encoding,
        stamp,
    })
}

fn undecoded(path: &Path) -> OpenError {
    OpenError::Undecoded(format!(
        "Cannot open {}: file is not valid UTF-8. reload <encoding> reads it, for example reload windows-874",
        path.display()
    ))
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, OpenError> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(OpenError::Missing),
        Err(err) => Err(OpenError::Failed(format!(
            "Cannot open {}: {err}",
            path.display()
        ))),
    }
}

fn detect(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(TextEncoding::Utf16Le)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(TextEncoding::Utf16Be)
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        std::str::from_utf8(&bytes[3..])
            .ok()
            .map(|_| TextEncoding::Utf8Bom)
    } else {
        std::str::from_utf8(bytes).ok().map(|_| TextEncoding::Utf8)
    }
}

fn strip_bom(bytes: &[u8], encoding: TextEncoding) -> &[u8] {
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) => {
            &bytes[3..]
        }
        TextEncoding::Utf16Le if bytes.starts_with(&[0xFF, 0xFE]) => &bytes[2..],
        TextEncoding::Utf16Be if bytes.starts_with(&[0xFE, 0xFF]) => &bytes[2..],
        _ => bytes,
    }
}

fn decode(bytes: &[u8], encoding: TextEncoding) -> Result<String, String> {
    let bytes = strip_bom(bytes, encoding);
    let invalid = || format!("file is not valid {}", encoding.label());
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            String::from_utf8(bytes.to_vec()).map_err(|_| invalid())
        }
        TextEncoding::Utf16Le => decode_utf16(bytes, true).map_err(|_| invalid()),
        TextEncoding::Utf16Be => decode_utf16(bytes, false).map_err(|_| invalid()),
        TextEncoding::Windows1252 => decode_legacy(bytes, encoding_rs::WINDOWS_1252, encoding),
        TextEncoding::Windows874 => decode_legacy(bytes, encoding_rs::WINDOWS_874, encoding),
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, ()> {
    if bytes.len() % 2 != 0 {
        return Err(());
    }
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| ())
}

fn encode(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, String> {
    match encoding {
        TextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        TextEncoding::Utf8Bom => {
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        TextEncoding::Utf16Le => Ok(encode_utf16(text, true)),
        TextEncoding::Utf16Be => Ok(encode_utf16(text, false)),
        TextEncoding::Windows1252 => encode_legacy(text, encoding_rs::WINDOWS_1252, encoding),
        TextEncoding::Windows874 => encode_legacy(text, encoding_rs::WINDOWS_874, encoding),
    }
}

fn decode_legacy(
    bytes: &[u8],
    table: &'static encoding_rs::Encoding,
    encoding: TextEncoding,
) -> Result<String, String> {
    let (text, _, had_errors) = table.decode(bytes);
    if had_errors {
        Err(format!("file is not valid {}", encoding.label()))
    } else {
        Ok(text.into_owned())
    }
}

fn encode_legacy(
    text: &str,
    table: &'static encoding_rs::Encoding,
    encoding: TextEncoding,
) -> Result<Vec<u8>, String> {
    let (bytes, _, had_errors) = table.encode(text);
    if had_errors {
        Err(format!("{} cannot represent this text", encoding.label()))
    } else {
        Ok(bytes.into_owned())
    }
}

fn encode_utf16(text: &str, little_endian: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + text.len() * 2);
    out.extend_from_slice(if little_endian {
        &[0xFF, 0xFE]
    } else {
        &[0xFE, 0xFF]
    });
    for unit in text.encode_utf16() {
        let bytes = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("duckx-encoding-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bom_and_explicit_encodings_roundtrip() {
        let dir = TempDir::new();
        let cases = [
            (TextEncoding::Utf8, "ก\n"),
            (TextEncoding::Utf8Bom, "ก\n"),
            (TextEncoding::Utf16Le, "กA\n"),
            (TextEncoding::Utf16Be, "กA\n"),
            (TextEncoding::Windows874, "ก\n"),
            (TextEncoding::Windows1252, "café\n"),
        ];
        for (encoding, text) in cases {
            let path = dir.0.join(encoding.label().replace(' ', "-"));
            write_encoded(&path, text, encoding).unwrap();
            let decoded = read_with(&path, encoding).unwrap();
            assert_eq!(decoded.text, text, "{}", encoding.label());
            assert_eq!(decoded.encoding, encoding);
            if matches!(
                encoding,
                TextEncoding::Utf8
                    | TextEncoding::Utf8Bom
                    | TextEncoding::Utf16Le
                    | TextEncoding::Utf16Be
            ) {
                let auto = read_auto(&path).unwrap();
                assert_eq!(auto.text, text, "{}", encoding.label());
                assert_eq!(auto.encoding, encoding, "{}", encoding.label());
            }
        }
    }

    #[test]
    fn bytes_without_a_bom_are_not_guessed() {
        let dir = TempDir::new();
        let path = dir.0.join("thai.txt");
        fs::write(&path, [0xA1]).unwrap();
        let err = read_auto(&path).unwrap_err();
        assert!(matches!(err, OpenError::Undecoded(_)), "{err:?}");
        let decoded = read_with(&path, TextEncoding::Windows874).unwrap();
        assert_eq!(decoded.text, "ก");
    }

    #[test]
    fn an_unmappable_character_is_not_encoded() {
        let err = encode("hi🙂", TextEncoding::Windows874).unwrap_err();
        assert!(err.contains("cannot represent"), "{err}");
        assert!(encode("ก", TextEncoding::Windows874).is_ok());
    }

    #[test]
    fn encoding_names_ignore_case_and_separators() {
        assert_eq!(
            TextEncoding::parse("UTF-16 LE"),
            Some(TextEncoding::Utf16Le)
        );
        assert_eq!(
            TextEncoding::parse("tis-620"),
            Some(TextEncoding::Windows874)
        );
        assert_eq!(TextEncoding::parse("latin1"), None);
    }
}
