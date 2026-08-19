//! Reading provenance back out of a manifest for `--check`.
//!
//! This is not a JSON parser. The manifests are written by this tool with a
//! fixed layout (`"key": "value"` with one space), the two values pulled back
//! out -- the fingerprint and the page image names -- are hex digits and file
//! names this tool also chose, and unknown files fail safe: no fingerprint
//! found means "stale", never "fine".

/// What `--check` needs from a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestInfo {
    /// The stored input fingerprint, if the file carries one.
    pub fingerprint: Option<String>,
    /// Every page image the manifest references, in order of appearance.
    pub images: Vec<String>,
}

/// Extract every string value stored under `"key": "..."`.
fn all_string_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\": \"");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(&needle) {
        let tail = &rest[pos + needle.len()..];
        if let Some(end) = tail.find('"') {
            out.push(tail[..end].to_string());
            rest = &tail[end..];
        } else {
            break;
        }
    }
    out
}

/// Scan a manifest this tool wrote.
pub fn scan_manifest(text: &str) -> ManifestInfo {
    ManifestInfo {
        fingerprint: all_string_values(text, "fingerprint").into_iter().next(),
        images: all_string_values(text, "image"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fingerprint_and_images() {
        let doc = r#"{
  "meta": {
    "image": "atlas-0.png",
    "sheetpack": { "fingerprint": "00ff00ff00ff00ff", "settings": "x" }
  },
  "more": { "image": "atlas-1.png" }
}"#;
        let info = scan_manifest(doc);
        assert_eq!(info.fingerprint.as_deref(), Some("00ff00ff00ff00ff"));
        assert_eq!(info.images, ["atlas-0.png", "atlas-1.png"]);
    }

    #[test]
    fn missing_fields_fail_safe() {
        let info = scan_manifest("{}");
        assert_eq!(info.fingerprint, None);
        assert!(info.images.is_empty());
    }
}
