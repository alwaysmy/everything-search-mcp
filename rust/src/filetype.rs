//! File type classification, in two deliberately separated levels.
//!
//! 1. `classify(name)` — extension and special-filename only. **Never touches the
//!    filesystem.** Used by search results. Reading a header per result would turn
//!    one index lookup into N filesystem operations, which behaves completely
//!    differently on spinning disks, SMB shares, cloud placeholders and machines
//!    with aggressive antivirus.
//! 2. `refine(info, bytes)` — magic/BOM/heuristic sniffing over a small header
//!    read. Only used by `everything_file_details`, and only when the extension is
//!    missing, unknown or genuinely ambiguous.
//!
//! `kind` is deliberately the same vocabulary as the `category` search parameter,
//! so a model can feed a `kind` straight back into `category` without translating.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Extensions whose content really cannot be inferred from the name.
const AMBIGUOUS: &[&str] = &[
    "dat", "bin", "db", "sqlite", "db3", "dmp", "dump", "raw", "img", "sav", "tmp", "cache",
];

/// Text-ish extensions that no category covers.
const TEXT_EXTS: &[&str] = &[
    "log", "nfo", "diff", "patch", "srt", "vtt", "tex", "bib", "org", "adoc", "asciidoc",
    "readme", "license", "authors", "changelog", "todo", "mk", "am", "ac", "m4", "in",
];

/// Extensions that belong to a binary category but are actually plain text.
const TEXT_DESPITE_KIND: &[&str] = &["svg", "ps", "eps", "plist"];

/// Extensionless files that are recognised by name.
const SPECIAL_NAMES: &[(&str, &str)] = &[
    ("makefile", "makefile"),
    ("gnumakefile", "makefile"),
    ("dockerfile", "dockerfile"),
    ("cmakelists.txt", "cmake"),
    ("cargo.toml", "toml"),
    ("cargo.lock", "toml"),
    ("package.json", "json"),
    ("pyproject.toml", "toml"),
    ("requirements.txt", "text"),
    ("go.mod", "gomod"),
    (".gitignore", "gitignore"),
    (".gitattributes", "gitattributes"),
    (".editorconfig", "editorconfig"),
    (".env", "env"),
    ("license", "text"),
    ("readme", "text"),
    ("changelog", "text"),
];

/// Friendly format names for the extensions people actually meet.
fn friendly_format(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "typescript-react",
        "jsx" => "javascript-react",
        "md" => "markdown",
        "yml" => "yaml",
        "htm" => "html",
        "c" => "c",
        "h" => "c-header",
        "cpp" | "cc" | "cxx" => "cpp",
        "hpp" => "cpp-header",
        "cs" => "csharp",
        "sh" => "shell",
        "bash" => "bash",
        "ps1" => "powershell",
        "bat" | "cmd" => "batch",
        "toml" => "toml",
        "json" => "json",
        "xml" => "xml",
        "csproj" => "msbuild",
        "uvprojx" => "keil-project",
        "uvoptx" => "keil-options",
        "eww" => "iar-workspace",
        "ewp" => "iar-project",
        "sln" => "vs-solution",
        "epro" => "lceda-project",
        other => other,
    }
}

/// `ext:` clause -> category, inverted once from the search category table.
fn ext_to_kind() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for (kind, clause) in crate::everything::FILE_TYPES {
            let list = clause.strip_prefix("ext:").unwrap_or(clause);
            for ext in list.split(';') {
                m.insert(ext, *kind);
            }
        }
        m
    })
}

/// Which category extensions are actually text, decided per category rather than
/// per extension: a `.md` is a document and it is text, while a `.docx` is a
/// document and it is not.
fn is_text_ext(ext: &str) -> bool {
    if TEXT_DESPITE_KIND.contains(&ext) {
        return true;
    }
    match ext_to_kind().get(ext) {
        // code and data categories are overwhelmingly text formats
        Some(&"code") | Some(&"data") => true,
        // documents are mixed: only the plain ones are text
        Some(&"document") => matches!(ext, "txt" | "md" | "rtf" | "epub" | "key"),
        // images are binary except the vector formats handled above
        _ => TEXT_EXTS.contains(&ext),
    }
}

#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// One of the search categories, plus `text` and `unknown`.
    pub kind: String,
    /// `text`, `binary`, or `unknown` when the name alone cannot say.
    pub content_mode: String,
    /// Friendly format name when known, otherwise the bare extension.
    pub format: String,
    /// How the classification was reached: `extension`, `magic` or `heuristic`.
    pub type_source: String,
}

impl TypeInfo {
    fn unknown() -> Self {
        TypeInfo {
            kind: "unknown".into(),
            content_mode: "unknown".into(),
            format: String::new(),
            type_source: "extension".into(),
        }
    }
}

/// Extension-only classification. Pure string work: no filesystem access.
pub fn classify(name: &str) -> TypeInfo {
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit(['\\', '/']).next().unwrap_or(&lower);

    if let Some((_, fmt)) = SPECIAL_NAMES.iter().find(|(n, _)| *n == base) {
        return TypeInfo {
            kind: "code".into(),
            content_mode: "text".into(),
            format: (*fmt).to_string(),
            type_source: "extension".into(),
        };
    }

    let ext = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext,
        // a leading dot (".gitignore") is a name, not an extension
        _ => {
            return if TEXT_EXTS.contains(&base) {
                TypeInfo {
                    kind: "text".into(),
                    content_mode: "text".into(),
                    format: base.to_string(),
                    type_source: "extension".into(),
                }
            } else {
                TypeInfo::unknown()
            }
        }
    };

    if AMBIGUOUS.contains(&ext) {
        return TypeInfo {
            kind: ext_to_kind().get(ext).copied().unwrap_or("unknown").to_string(),
            content_mode: "unknown".into(),
            format: friendly_format(ext).to_string(),
            type_source: "extension".into(),
        };
    }

    let kind = match ext_to_kind().get(ext) {
        Some(k) => (*k).to_string(),
        None if TEXT_EXTS.contains(&ext) => "text".to_string(),
        None => "unknown".to_string(),
    };
    let content_mode = if kind == "unknown" {
        "unknown"
    } else if is_text_ext(ext) {
        "text"
    } else {
        "binary"
    };

    TypeInfo {
        kind,
        content_mode: content_mode.to_string(),
        format: friendly_format(ext).to_string(),
        type_source: "extension".into(),
    }
}

/// Sniff a small header. Order matters: BOM first, then magic, then a text
/// heuristic. A NUL byte alone does NOT mean binary — UTF-16 text is full of them.
pub fn refine(mut info: TypeInfo, bytes: &[u8]) -> TypeInfo {
    // 1. byte-order marks
    let bom_text = bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]);
    if bom_text {
        info.content_mode = "text".into();
        if info.kind == "unknown" {
            info.kind = "text".into();
        }
        info.type_source = "magic".into();
        return info;
    }

    // 2. reliable magic numbers
    if let Some((mode, fmt)) = magic(bytes) {
        info.content_mode = mode.into();
        if let Some(f) = fmt {
            info.format = f.to_string();
            if info.kind == "unknown" {
                info.kind = ext_to_kind()
                    .get(f)
                    .copied()
                    .unwrap_or(if mode == "text" { "text" } else { "unknown" })
                    .to_string();
            }
        }
        info.type_source = "magic".into();
        return info;
    }

    // 3. UTF-16 without a BOM: NULs on a regular stride, which must be checked
    //    before any "contains NUL => binary" shortcut.
    if looks_utf16(bytes) {
        info.content_mode = "text".into();
        if info.kind == "unknown" {
            info.kind = "text".into();
        }
        info.type_source = "heuristic".into();
        return info;
    }

    // 4. UTF-8 validity plus a control-character ratio
    if !bytes.is_empty() {
        let sample = &bytes[..bytes.len().min(8192)];
        let control = sample
            .iter()
            .filter(|b| **b < 0x09 || (**b > 0x0D && **b < 0x20))
            .count();
        let utf8_ok = std::str::from_utf8(sample).is_ok()
            || std::str::from_utf8(&sample[..sample.len().saturating_sub(3)]).is_ok();
        let ratio = control as f64 / sample.len() as f64;
        if utf8_ok && ratio < 0.05 {
            info.content_mode = "text".into();
            if info.kind == "unknown" {
                info.kind = "text".into();
            }
            info.type_source = "heuristic".into();
        } else {
            info.content_mode = "binary".into();
            info.type_source = "heuristic".into();
        }
    }
    info
}

fn magic(b: &[u8]) -> Option<(&'static str, Option<&'static str>)> {
    let starts = |sig: &[u8]| b.len() >= sig.len() && &b[..sig.len()] == sig;
    if starts(&[0x89, b'P', b'N', b'G']) {
        return Some(("binary", Some("png")));
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(("binary", Some("jpg")));
    }
    if starts(b"GIF8") {
        return Some(("binary", Some("gif")));
    }
    if starts(b"%PDF") {
        return Some(("binary", Some("pdf")));
    }
    if starts(b"PK\x03\x04") {
        return Some(("binary", Some("zip")));
    }
    if starts(b"Rar!") {
        return Some(("binary", Some("rar")));
    }
    if starts(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some(("binary", Some("7z")));
    }
    if starts(&[0x1F, 0x8B]) {
        return Some(("binary", Some("gz")));
    }
    if starts(b"MZ") {
        return Some(("binary", Some("exe")));
    }
    if starts(&[0x7F, b'E', b'L', b'F']) {
        return Some(("binary", Some("elf")));
    }
    if starts(b"SQLite format 3") {
        return Some(("binary", Some("sqlite")));
    }
    if starts(b"RIFF") {
        let fourcc = b.get(8..12).unwrap_or(b"");
        let fmt = match fourcc {
            b"WAVE" => Some("wav"),
            b"AVI " => Some("avi"),
            b"WEBP" => Some("webp"),
            _ => None,
        };
        return Some(("binary", fmt));
    }
    if b.len() > 12 && &b[4..8] == b"ftyp" {
        let brand = &b[8..12];
        let fmt = match brand {
            b"isom" | b"mp42" | b"avc1" => Some("mp4"),
            b"M4A " => Some("m4a"),
            b"qt  " => Some("mov"),
            _ => Some("mp4"),
        };
        return Some(("binary", fmt));
    }
    if starts(b"{\\rtf") {
        return Some(("text", Some("rtf")));
    }
    if starts(b"<?xml") {
        return Some(("text", Some("xml")));
    }
    if starts(b"<svg") || (b.len() > 200 && String::from_utf8_lossy(&b[..200]).contains("<svg")) {
        return Some(("text", Some("svg")));
    }
    None
}

fn looks_utf16(b: &[u8]) -> bool {
    let n = b.len().min(512);
    if n < 8 {
        return false;
    }
    let even_nul = (0..n).step_by(2).filter(|i| b[*i] == 0).count();
    let odd_nul = (1..n).step_by(2).filter(|i| b[*i] == 0).count();
    let pairs = n / 2;
    // most NULs on one side, none on the other => UTF-16
    (even_nul * 10 > pairs * 8 && odd_nul * 20 < pairs)
        || (odd_nul * 10 > pairs * 8 && even_nul * 20 < pairs)
}

/// The header size that is enough to sniff reliably, and cheap enough to never
/// matter.
pub const SNIFF_BYTES: usize = 16 * 1024;

/// Text encoding implied by a header. Used to decode previews: a UTF-16 file
/// decoded as UTF-8 renders as text with gaps between every character.
/// PowerShell's `>` redirection produces UTF-16LE by default, so this is not rare
/// on Windows.
pub fn encoding_of(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return "utf-8";
    }
    if bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        return "utf-32le";
    }
    if bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        return "utf-32be";
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return "utf-16le";
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return "utf-16be";
    }
    if looks_utf16(bytes) {
        let n = bytes.len().min(512);
        let even_nul = (0..n).step_by(2).filter(|i| bytes[*i] == 0).count();
        let odd_nul = (1..n).step_by(2).filter(|i| bytes[*i] == 0).count();
        return if odd_nul > even_nul { "utf-16le" } else { "utf-16be" };
    }
    "utf-8"
}

/// Decode a text buffer using the encoding the header implies.
pub fn decode_text(bytes: &[u8]) -> String {
    match encoding_of(bytes) {
        "utf-16le" => String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ),
        "utf-16be" => String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ),
        _ => String::from_utf8_lossy(bytes)
            .trim_start_matches('\u{feff}')
            .to_string(),
    }
}
