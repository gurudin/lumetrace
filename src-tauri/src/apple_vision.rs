//! On-device Apple Vision. No model downloads, network calls, or source writes.
use serde::Deserialize;
use std::{
    ffi::{CStr, CString},
    path::Path,
};

#[derive(Debug, Deserialize)]
pub(crate) struct Recognition {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<Classification>,
    #[serde(default)]
    #[allow(dead_code)] // Used by native regression checks, not application UI.
    pub ocr_pages: usize,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Classification {
    pub identifier: String,
    pub confidence: f32,
}

unsafe extern "C" {
    fn lumetrace_vision_available() -> bool;
    fn lumetrace_vision_read(path: *const std::ffi::c_char, pdf: bool) -> *mut std::ffi::c_char;
    fn lumetrace_vision_free(result: *mut std::ffi::c_char);
}

pub(crate) fn available() -> bool {
    unsafe { lumetrace_vision_available() }
}

pub(crate) fn recognize(path: &Path, pdf: bool) -> Result<Recognition, String> {
    use std::os::unix::ffi::OsStrExt;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().map_err(|_| "Apple Vision queue unavailable")?;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| "Invalid recognition path")?;
    // Native owns the buffer until copied, and catches Objective-C exceptions.
    let result = unsafe { lumetrace_vision_read(path.as_ptr(), pdf) };
    if result.is_null() {
        return Err("Apple Vision returned no result".into());
    }
    let bytes = unsafe { CStr::from_ptr(result) }.to_bytes().to_vec();
    unsafe { lumetrace_vision_free(result) };
    let value: Recognition =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Apple Vision result")?;
    if let Some(error) = &value.error {
        return Err(error.clone());
    }
    Ok(value)
}

pub(crate) fn extract(path: &Path, pdf: bool) -> Result<String, String> {
    let result = recognize(path, pdf)?;
    let mut text = result.body;
    let labels = classification_text(&result.labels);
    if !labels.is_empty() {
        text.push_str("\n\n[Image categories — automatically inferred, not document text]\n");
        text.push_str(&labels);
    }
    Ok(text)
}

fn classification_text(labels: &[Classification]) -> String {
    let mut words = Vec::new();
    for label in labels
        .iter()
        .filter(|v| v.confidence.is_finite() && v.confidence >= 0.3)
        .take(8)
    {
        let normalized = label.identifier.replace('_', " ");
        if !words.contains(&normalized) {
            words.push(normalized.clone());
        }
        // Small offline vocabulary for common objects in all eight app languages.
        // Unknown system categories retain their English identifier. No online
        // translation, fabricated descriptions, user tags, or filename changes.
        for line in include_str!("vision_labels.tsv")
            .lines()
            .filter(|line| !line.starts_with('#'))
        {
            let Some((identifiers, aliases)) = line.split_once('\t') else {
                continue;
            };
            if identifiers.split('|').any(|id| {
                label.identifier == id || normalized.split_whitespace().any(|word| word == id)
            }) {
                let aliases = aliases.to_owned();
                if !words.contains(&aliases) {
                    words.push(aliases);
                }
            }
        }
    }
    words.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_are_localized_but_not_invented_or_substring_matched() {
        let labels = vec![
            Classification {
                identifier: "adult_cat".into(),
                confidence: 0.9,
            },
            Classification {
                identifier: "catfish".into(),
                confidence: 0.1,
            },
        ];
        let text = classification_text(&labels);
        assert!(text.contains("猫") && text.contains("chat") && text.contains("cat"));
        assert!(!text.contains("catfish"));
        assert!(!classification_text(&[Classification {
            identifier: "catfish".into(),
            confidence: 0.9
        }])
        .contains("猫"));
        assert!(classification_text(&[Classification {
            identifier: "cat".into(),
            confidence: f32::NAN
        }])
        .is_empty());
    }

    #[test]
    #[ignore = "requires explicitly generated synthetic LUMETRACE_TEST_VISION_DIR fixtures; uses real Apple Vision"]
    fn native_vision_reads_images_scans_and_mixed_pdf_without_changing_sources() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(
            std::env::var_os("LUMETRACE_TEST_VISION_DIR")
                .expect("synthetic fixture directory required"),
        );
        for name in [
            "document.png",
            "document.jpg",
            "document.tiff",
            "scan.pdf",
            "mixed.pdf",
        ] {
            let path = root.join(name);
            let before = Sha256::digest(std::fs::read(&path).unwrap());
            let result = recognize(&path, name.ends_with(".pdf")).unwrap();
            eprintln!(
                "{name}: OCR pages={}, categories={:?}, text={}",
                result.ocr_pages, result.labels, result.body
            );
            assert!(
                result.body.contains("ORCHID") && result.body.contains("September 20"),
                "{result:?}"
            );
            assert!(
                result.body.contains("项目预算") && result.body.contains("北京"),
                "{result:?}"
            );
            assert_eq!(result.ocr_pages, 1);
            if name == "mixed.pdf" {
                assert!(result.body.contains("TEXTLAYER QUARTZ"));
                assert!(result.body.contains("[Page 2]"));
            }
            if !name.ends_with(".pdf") {
                assert!(
                    result
                        .labels
                        .iter()
                        .any(|v| v.identifier == "document" || v.identifier == "printed_page"),
                    "{result:?}"
                );
            }
            assert_eq!(before, Sha256::digest(std::fs::read(path).unwrap()));
        }
    }
}
