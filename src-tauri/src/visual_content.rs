//! Cached display geometry, separate from the text consumed by FTS and E5.
use serde::{Deserialize, Serialize};

pub(crate) const CATEGORY_MARKER: &str =
    "\n\n[Image categories — automatically inferred, not document text]\n";

/// A category returned by the system image classifier. These labels are
/// searchable metadata, not claimed object coordinates.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct VisualLabel {
    pub identifier: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisualRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl VisualRect {
    pub(crate) fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1.001
            && self.y + self.height <= 1.001
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct VisualWord {
    /// UTF-16 offsets in the original line, as returned by Apple's text API.
    pub start: usize,
    pub end: usize,
    pub rect: VisualRect,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct VisualLine {
    pub text: String,
    pub rect: VisualRect,
    #[serde(default)]
    pub words: Vec<VisualWord>,
}

impl VisualLine {
    pub(crate) fn rectangles(&self, start: usize, end: usize) -> Vec<VisualRect> {
        let start = self
            .text
            .chars()
            .take(start)
            .map(char::len_utf16)
            .sum::<usize>();
        let end = self
            .text
            .chars()
            .take(end)
            .map(char::len_utf16)
            .sum::<usize>();
        let boxes: Vec<_> = self
            .words
            .iter()
            .filter(|w| w.start < end && w.end > start && w.rect.valid())
            .map(|w| w.rect.clone())
            .collect();
        if boxes.is_empty() && self.rect.valid() {
            vec![self.rect.clone()]
        } else {
            boxes
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct VisualPage {
    pub number: usize,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<VisualLine>,
    /// Categories inferred from raster content on this PDF page.
    #[serde(default)]
    pub labels: Vec<VisualLabel>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct VisualDocument {
    pub pages: Vec<VisualPage>,
}

pub(crate) fn display_body(body: &str) -> &str {
    body.split_once(CATEGORY_MARKER)
        .map_or(body, |(text, _)| text)
}

pub(crate) fn is_raster(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_rejects_invalid_boxes_and_maps_unicode_offsets() {
        let rect = VisualRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.1,
        };
        let line = VisualLine {
            text: "😀预算".into(),
            rect: rect.clone(),
            words: vec![VisualWord {
                start: 2,
                end: 3,
                rect: rect.clone(),
            }],
        };
        assert_eq!(line.rectangles(1, 2), vec![rect.clone()]);
        assert_eq!(line.rectangles(2, 3), vec![rect]); // precise geometry unavailable: line fallback
        assert!(!VisualRect {
            x: f64::NAN,
            ..Default::default()
        }
        .valid());
        assert_eq!(display_body(&format!("OCR{CATEGORY_MARKER}cat 猫")), "OCR");
    }

    #[test]
    fn visual_pages_keep_labels_and_accept_older_cached_json() {
        let older = r#"{"pages":[{"number":1,"width":100.0,"height":80.0,"lines":[]}]}"#;
        let document: VisualDocument = serde_json::from_str(older).unwrap();
        assert!(document.pages[0].labels.is_empty());

        let document = VisualDocument {
            pages: vec![VisualPage {
                number: 1,
                width: 100.0,
                height: 80.0,
                lines: Vec::new(),
                labels: vec![VisualLabel {
                    identifier: "cake".into(),
                    confidence: 0.82,
                }],
            }],
        };
        let restored: VisualDocument =
            serde_json::from_str(&serde_json::to_string(&document).unwrap()).unwrap();
        assert_eq!(restored.pages[0].labels[0].identifier, "cake");
    }
}
