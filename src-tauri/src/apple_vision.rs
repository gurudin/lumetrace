//! On-device Apple Vision. No model downloads, network calls, or source writes.
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, CString},
    path::Path,
};

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Recognition {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<Classification>,
    #[serde(default)]
    pub pages: Vec<crate::visual_content::VisualPage>,
    #[serde(default)]
    #[allow(dead_code)] // Used by native regression checks, not application UI.
    pub ocr_pages: usize,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
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
    extract_with_geometry(path, pdf).map(|(text, _)| text)
}

pub(crate) fn extract_with_geometry(
    path: &Path,
    pdf: bool,
) -> Result<(String, crate::visual_content::VisualDocument), String> {
    let result = recognize(path, pdf)?;
    let mut text = result.body;
    let labels = classification_text(&result.labels);
    if !labels.is_empty() {
        text.push_str(crate::visual_content::CATEGORY_MARKER);
        text.push_str(&labels);
    }
    Ok((
        text,
        crate::visual_content::VisualDocument {
            pages: result.pages,
        },
    ))
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
#[test]
#[ignore = "Requires synthetic Vision fixtures and real macOS frameworks"]
fn native_vision_visual_geometry_covers_embedded_images_and_rotated_pages() {
    let root = std::path::PathBuf::from(
        std::env::var_os("LUMETRACE_TEST_VISION_DIR").expect("fixture directory"),
    );
    for name in [
        "document.png",
        "scan.pdf",
        "mixed.pdf",
        "embedded.pdf",
        "rotated.pdf",
    ] {
        let result = recognize(&root.join(name), name.ends_with(".pdf")).unwrap();
        assert!(result.body.contains("ORCHID"), "{name}: {}", result.body);
        if name == "embedded.pdf" || name == "rotated.pdf" {
            assert!(result.body.contains("NATIVE HEADER QUARTZ"));
            assert_eq!(
                result.ocr_pages, 1,
                "Text-bearing image pages must also be OCRed"
            );
        }
        let page = result
            .pages
            .iter()
            .find(|p| p.lines.iter().any(|l| l.text.contains("ORCHID")))
            .expect("OCR page geometry");
        let line = page
            .lines
            .iter()
            .find(|l| l.text.contains("ORCHID"))
            .unwrap();
        assert!(line.rect.valid());
        assert!(
            !line.words.is_empty(),
            "Substring coordinates missing: {name}"
        );
        assert!(line.words.iter().all(|w| w.rect.valid()
            && w.start < w.end
            && w.end <= line.text.encode_utf16().count()));
        if name == "mixed.pdf" {
            assert_eq!(page.number, 2);
        }
        if name == "rotated.pdf" {
            assert!(page.height > page.width);
        }
        if name == "embedded.pdf" {
            assert!(
                line.rect.y > 0.5,
                "Embedded OCR must be below the native header"
            );
        }
        if name == "document.png" || name == "scan.pdf" {
            let boxes = line.rectangles(0, 6);
            let left = boxes.iter().map(|r| r.x).fold(1.0, f64::min);
            let right = boxes.iter().map(|r| r.x + r.width).fold(0.0, f64::max);
            assert!(
                left < 0.06 && right < 0.3 && right > 0.15,
                "ORCHID boxes: {boxes:?}"
            );
        }
        // Optional evidence for the synthetic browser fixture; never user data.
        if std::env::var_os("LUMETRACE_TEST_WRITE_GEOMETRY").is_some() {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(format!("{name}.vision.json")))
                .unwrap();
            file.write_all(serde_json::to_string(&result).unwrap().as_bytes())
                .unwrap();
        }
    }
}
