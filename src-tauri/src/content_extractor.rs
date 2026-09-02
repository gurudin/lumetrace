use quick_xml::{
    escape::unescape,
    events::{BytesRef, BytesStart, BytesText, Event},
    Reader, XmlVersion,
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Cursor, Read},
    path::Path,
};
use zip::ZipArchive;

pub(crate) const EXTRACTION_VERSION: i64 = 1;
const MAX_SOURCE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_CHARACTERS: usize = 5_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtractionStatus {
    Pending,
    Extracted,
    Empty,
    Unsupported,
    Failed,
}

impl ExtractionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Extracted => "extracted",
            Self::Empty => "empty",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentExtraction {
    pub(crate) text: String,
    pub(crate) status: ExtractionStatus,
    pub(crate) error: Option<String>,
}

impl ContentExtraction {
    fn from_result(result: Result<String, String>) -> Self {
        match result {
            Ok(text) => {
                let text = normalize_extracted_text(text);
                let status = if text.is_empty() {
                    ExtractionStatus::Empty
                } else {
                    ExtractionStatus::Extracted
                };
                Self {
                    text,
                    status,
                    error: None,
                }
            }
            Err(error) => Self {
                text: String::new(),
                status: ExtractionStatus::Failed,
                error: Some(error),
            },
        }
    }

    fn unsupported() -> Self {
        Self {
            text: String::new(),
            status: ExtractionStatus::Unsupported,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    PlainText,
    Pdf,
    Docx,
    Xlsx,
    Pptx,
}

fn content_kind(name: &str) -> Option<ContentKind> {
    let path = Path::new(name);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("pdf") => Some(ContentKind::Pdf),
        Some("docx") => Some(ContentKind::Docx),
        Some("xlsx") => Some(ContentKind::Xlsx),
        Some("pptx") => Some(ContentKind::Pptx),
        Some(
            "md" | "markdown" | "mdx" | "txt" | "text" | "log" | "csv" | "tsv" | "rst" | "adoc"
            | "asciidoc" | "json" | "jsonl" | "ndjson" | "xml" | "yaml" | "yml" | "toml" | "ini"
            | "conf" | "cfg" | "properties" | "html" | "htm" | "css" | "scss" | "sass" | "less"
            | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "py" | "rs" | "go" | "java" | "kt"
            | "kts" | "swift" | "c" | "cc" | "cpp" | "h" | "hpp" | "cs" | "rb" | "php" | "sh"
            | "bash" | "zsh" | "fish" | "sql" | "vue" | "svelte" | "graphql" | "gql" | "svg",
        ) => Some(ContentKind::PlainText),
        None if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "dockerfile"
                        | "makefile"
                        | "gemfile"
                        | "rakefile"
                        | ".gitignore"
                        | ".gitattributes"
                        | ".editorconfig"
                        | ".env"
                )
            }) =>
        {
            Some(ContentKind::PlainText)
        }
        _ => None,
    }
}

pub(crate) fn extract_file_content(path: &Path, name: &str) -> ContentExtraction {
    let Some(kind) = content_kind(name) else {
        return ContentExtraction::unsupported();
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return ContentExtraction::from_result(Err(format!(
                "Unable to inspect the file for content extraction: {error}"
            )))
        }
    };
    if !metadata.is_file() {
        return ContentExtraction::from_result(Err(
            "The content extraction target is not a regular file".to_owned(),
        ));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return ContentExtraction::from_result(Err(format!(
            "The file is larger than the {} MB content extraction limit",
            MAX_SOURCE_BYTES / 1024 / 1024
        )));
    }
    let result = match kind {
        ContentKind::PlainText => read_plain_text(path),
        ContentKind::Pdf => extract_pdf(path),
        ContentKind::Docx => extract_docx(path),
        ContentKind::Xlsx => extract_xlsx(path),
        ContentKind::Pptx => extract_pptx(path),
    };
    ContentExtraction::from_result(result)
}

fn read_plain_text(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("Unable to read text content: {error}"))
}

fn extract_pdf(path: &Path) -> Result<String, String> {
    let pages = pdf_extract::extract_text_by_pages(path)
        .map_err(|error| format!("Unable to extract PDF text: {error}"))?;
    let mut output = String::new();
    for (index, text) in pages.into_iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("[Page {}]\n", index + 1));
        output.push_str(text.trim());
    }
    Ok(output)
}

fn normalize_extracted_text(text: String) -> String {
    let mut normalized = text
        .replace('\0', "")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    while normalized.contains("\n\n\n") {
        normalized = normalized.replace("\n\n\n", "\n\n");
    }
    let normalized = normalized.trim();
    if normalized.chars().count() <= MAX_EXTRACTED_CHARACTERS {
        normalized.to_owned()
    } else {
        normalized
            .chars()
            .take(MAX_EXTRACTED_CHARACTERS)
            .collect::<String>()
    }
}

fn open_archive(path: &Path) -> Result<ZipArchive<File>, String> {
    let file =
        File::open(path).map_err(|error| format!("Unable to open Office document: {error}"))?;
    ZipArchive::new(file)
        .map_err(|error| format!("Unable to read Office document package: {error}"))
}

fn read_archive_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format!("Office document is missing {name}: {error}"))?;
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "Office document entry {name} is too large to extract safely"
        ));
    }
    let mut bytes = Vec::with_capacity(entry.size().min(1024 * 1024) as usize);
    entry
        .by_ref()
        .take(MAX_ARCHIVE_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Unable to read Office document entry {name}: {error}"))?;
    if bytes.len() as u64 > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "Office document entry {name} is too large to extract safely"
        ));
    }
    Ok(bytes)
}

fn decode_xml_text(text: &BytesText<'_>) -> Result<String, String> {
    let decoded = text
        .xml10_content()
        .map_err(|error| format!("Unable to decode Office document text: {error}"))?;
    unescape(decoded.as_ref())
        .map(|value| value.into_owned())
        .map_err(|error| format!("Unable to unescape Office document text: {error}"))
}

fn decode_xml_reference(reference: &BytesRef<'_>) -> Result<String, String> {
    if let Some(value) = reference
        .resolve_char_ref()
        .map_err(|error| format!("Unable to decode Office document character reference: {error}"))?
    {
        return Ok(value.to_string());
    }
    let name = reference
        .decode()
        .map_err(|error| format!("Unable to decode Office document entity reference: {error}"))?;
    Ok(match name.as_ref() {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "apos" => "'".to_owned(),
        "quot" => "\"".to_owned(),
        value => format!("&{value};"),
    })
}

fn attribute_value(
    reader: &Reader<Cursor<&[u8]>>,
    start: &BytesStart<'_>,
    local_name: &[u8],
) -> Result<Option<String>, String> {
    for attribute in start.attributes().with_checks(false) {
        let attribute =
            attribute.map_err(|error| format!("Invalid Office document attribute: {error}"))?;
        if attribute.key.local_name().as_ref() == local_name {
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|error| format!("Unable to decode Office document attribute: {error}"))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn append_separator(output: &mut String, separator: char) {
    if output.is_empty() || output.ends_with(separator) {
        return;
    }
    if separator == '\n' && output.ends_with('\t') {
        output.pop();
    }
    output.push(separator);
}

fn extract_docx(path: &Path) -> Result<String, String> {
    let mut archive = open_archive(path)?;
    let xml = read_archive_entry(&mut archive, "word/document.xml")?;
    extract_wordprocessing_xml(&xml)
}

fn extract_wordprocessing_xml(xml: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = String::new();
    let mut inside_text = false;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse DOCX content: {error}"))?
        {
            Event::Start(start) => {
                inside_text = start.name().local_name().as_ref() == b"t";
            }
            Event::Empty(empty) => match empty.name().local_name().as_ref() {
                b"tab" => append_separator(&mut output, '\t'),
                b"br" | b"cr" => append_separator(&mut output, '\n'),
                _ => {}
            },
            Event::Text(text) if inside_text => output.push_str(&decode_xml_text(&text)?),
            Event::GeneralRef(reference) if inside_text => {
                output.push_str(&decode_xml_reference(&reference)?)
            }
            Event::CData(text) if inside_text => output.push_str(
                text.decode()
                    .map_err(|error| format!("Unable to decode DOCX content: {error}"))?
                    .as_ref(),
            ),
            Event::End(end) => match end.name().local_name().as_ref() {
                b"t" => inside_text = false,
                b"p" | b"tr" => append_separator(&mut output, '\n'),
                b"tc" => append_separator(&mut output, '\t'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(output)
}

fn extract_pptx(path: &Path) -> Result<String, String> {
    let mut archive = open_archive(path)?;
    let mut slides = archive
        .file_names()
        .filter_map(|name| {
            let suffix = name
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?;
            suffix
                .parse::<usize>()
                .ok()
                .map(|number| (number, name.to_owned()))
        })
        .collect::<Vec<_>>();
    slides.sort_by_key(|(number, _)| *number);
    let mut output = String::new();
    for (number, name) in slides {
        let xml = read_archive_entry(&mut archive, &name)?;
        let text = extract_presentation_xml(&xml)?;
        if text.trim().is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("[Slide {number}]\n"));
        output.push_str(text.trim());
    }
    Ok(output)
}

fn extract_presentation_xml(xml: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = String::new();
    let mut inside_text = false;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse PPTX content: {error}"))?
        {
            Event::Start(start) => inside_text = start.name().local_name().as_ref() == b"t",
            Event::Text(text) if inside_text => output.push_str(&decode_xml_text(&text)?),
            Event::GeneralRef(reference) if inside_text => {
                output.push_str(&decode_xml_reference(&reference)?)
            }
            Event::CData(text) if inside_text => output.push_str(
                text.decode()
                    .map_err(|error| format!("Unable to decode PPTX content: {error}"))?
                    .as_ref(),
            ),
            Event::End(end) => match end.name().local_name().as_ref() {
                b"t" => inside_text = false,
                b"p" => append_separator(&mut output, '\n'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(output)
}

#[derive(Debug)]
struct WorkbookSheet {
    name: String,
    relationship_id: String,
}

fn extract_xlsx(path: &Path) -> Result<String, String> {
    let mut archive = open_archive(path)?;
    let workbook_xml = read_archive_entry(&mut archive, "xl/workbook.xml")?;
    let relationship_xml = read_archive_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_workbook_sheets(&workbook_xml)?;
    let relationships = parse_workbook_relationships(&relationship_xml)?;
    let shared_strings = match read_archive_entry(&mut archive, "xl/sharedStrings.xml") {
        Ok(xml) => parse_shared_strings(&xml)?,
        Err(_) => Vec::new(),
    };
    let mut output = String::new();
    for sheet in sheets {
        let Some(target) = relationships.get(&sheet.relationship_id) else {
            continue;
        };
        let entry_name = resolve_archive_target("xl", target);
        let xml = read_archive_entry(&mut archive, &entry_name)?;
        let text = extract_worksheet_xml(&xml, &shared_strings)?;
        if text.trim().is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("[Sheet: {}]\n", sheet.name));
        output.push_str(text.trim());
    }
    Ok(output)
}

fn parse_workbook_sheets(xml: &[u8]) -> Result<Vec<WorkbookSheet>, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    let mut buffer = Vec::new();
    let mut sheets = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse XLSX workbook: {error}"))?
        {
            Event::Start(start) | Event::Empty(start)
                if start.name().local_name().as_ref() == b"sheet" =>
            {
                let name = attribute_value(&reader, &start, b"name")?.unwrap_or_default();
                let relationship_id = attribute_value(&reader, &start, b"id")?.unwrap_or_default();
                if !name.is_empty() && !relationship_id.is_empty() {
                    sheets.push(WorkbookSheet {
                        name,
                        relationship_id,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(sheets)
}

fn parse_workbook_relationships(xml: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    let mut buffer = Vec::new();
    let mut relationships = HashMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse XLSX relationships: {error}"))?
        {
            Event::Start(start) | Event::Empty(start)
                if start.name().local_name().as_ref() == b"Relationship" =>
            {
                let id = attribute_value(&reader, &start, b"Id")?.unwrap_or_default();
                let target = attribute_value(&reader, &start, b"Target")?.unwrap_or_default();
                if !id.is_empty() && !target.is_empty() {
                    relationships.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(relationships)
}

fn resolve_archive_target(base: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_owned();
    }
    let target = target.trim_start_matches('/');
    let mut parts = base
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value.to_owned()),
        }
    }
    parts.join("/")
}

fn parse_shared_strings(xml: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut inside_item = false;
    let mut inside_text = false;
    let mut phonetic_depth = 0usize;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse XLSX shared strings: {error}"))?
        {
            Event::Start(start) => match start.name().local_name().as_ref() {
                b"si" => {
                    inside_item = true;
                    current.clear();
                }
                b"rPh" => phonetic_depth += 1,
                b"t" if inside_item && phonetic_depth == 0 => inside_text = true,
                _ => {}
            },
            Event::Text(text) if inside_text => current.push_str(&decode_xml_text(&text)?),
            Event::GeneralRef(reference) if inside_text => {
                current.push_str(&decode_xml_reference(&reference)?)
            }
            Event::CData(text) if inside_text => current.push_str(
                text.decode()
                    .map_err(|error| format!("Unable to decode XLSX shared string: {error}"))?
                    .as_ref(),
            ),
            Event::End(end) => match end.name().local_name().as_ref() {
                b"t" => inside_text = false,
                b"rPh" => phonetic_depth = phonetic_depth.saturating_sub(1),
                b"si" => {
                    strings.push(current.clone());
                    inside_item = false;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(strings)
}

#[derive(Default)]
struct WorksheetCell {
    reference: String,
    value_type: String,
    value: String,
    inline_text: String,
    formula: String,
}

fn extract_worksheet_xml(xml: &[u8], shared_strings: &[String]) -> Result<String, String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = String::new();
    let mut cell: Option<WorksheetCell> = None;
    let mut capture_value = false;
    let mut capture_inline = false;
    let mut capture_formula = false;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("Unable to parse XLSX worksheet: {error}"))?
        {
            Event::Start(start) => match start.name().local_name().as_ref() {
                b"c" => {
                    cell = Some(WorksheetCell {
                        reference: attribute_value(&reader, &start, b"r")?.unwrap_or_default(),
                        value_type: attribute_value(&reader, &start, b"t")?.unwrap_or_default(),
                        ..WorksheetCell::default()
                    });
                }
                b"v" if cell.is_some() => capture_value = true,
                b"t" if cell.is_some() => capture_inline = true,
                b"f" if cell.is_some() => capture_formula = true,
                _ => {}
            },
            Event::Text(text) => {
                let value = decode_xml_text(&text)?;
                if let Some(cell) = cell.as_mut() {
                    if capture_value {
                        cell.value.push_str(&value);
                    } else if capture_inline {
                        cell.inline_text.push_str(&value);
                    } else if capture_formula {
                        cell.formula.push_str(&value);
                    }
                }
            }
            Event::CData(text) => {
                let value = text
                    .decode()
                    .map_err(|error| format!("Unable to decode XLSX worksheet: {error}"))?;
                if let Some(cell) = cell.as_mut() {
                    if capture_value {
                        cell.value.push_str(value.as_ref());
                    } else if capture_inline {
                        cell.inline_text.push_str(value.as_ref());
                    } else if capture_formula {
                        cell.formula.push_str(value.as_ref());
                    }
                }
            }
            Event::GeneralRef(reference) => {
                let value = decode_xml_reference(&reference)?;
                if let Some(cell) = cell.as_mut() {
                    if capture_value {
                        cell.value.push_str(&value);
                    } else if capture_inline {
                        cell.inline_text.push_str(&value);
                    } else if capture_formula {
                        cell.formula.push_str(&value);
                    }
                }
            }
            Event::End(end) => match end.name().local_name().as_ref() {
                b"v" => capture_value = false,
                b"t" => capture_inline = false,
                b"f" => capture_formula = false,
                b"c" => {
                    if let Some(cell) = cell.take() {
                        let value = worksheet_cell_value(&cell, shared_strings);
                        if !value.trim().is_empty() {
                            if !output.is_empty() && !output.ends_with('\n') {
                                output.push('\t');
                            }
                            if !cell.reference.is_empty() {
                                output.push_str(&cell.reference);
                                output.push_str(": ");
                            }
                            output.push_str(value.trim());
                        }
                    }
                }
                b"row" => append_separator(&mut output, '\n'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(output)
}

fn worksheet_cell_value(cell: &WorksheetCell, shared_strings: &[String]) -> String {
    match cell.value_type.as_str() {
        "s" => cell
            .value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared_strings.get(index))
            .cloned()
            .unwrap_or_default(),
        "inlineStr" => cell.inline_text.clone(),
        "b" => match cell.value.trim() {
            "1" => "true".to_owned(),
            "0" => "false".to_owned(),
            value => value.to_owned(),
        },
        _ if !cell.value.trim().is_empty() => cell.value.clone(),
        _ if !cell.inline_text.trim().is_empty() => cell.inline_text.clone(),
        _ if !cell.formula.trim().is_empty() => format!("={}", cell.formula.trim()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn temporary_path(extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lumetrace-content-extraction-{}.{}",
            Uuid::new_v4(),
            extension
        ))
    }

    fn write_office_package(extension: &str, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = temporary_path(extension);
        let file = File::create(&path).unwrap();
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, body) in entries {
            archive.start_file(*name, options).unwrap();
            archive.write_all(body).unwrap();
        }
        archive.finish().unwrap();
        path
    }

    fn simple_pdf_bytes(text: &str) -> Vec<u8> {
        simple_pdf_bytes_with_content(&format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET"))
    }

    fn simple_pdf_bytes_with_content(content: &str) -> Vec<u8> {
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn recognizes_plain_text_and_requested_document_formats() {
        assert_eq!(content_kind("notes.md"), Some(ContentKind::PlainText));
        assert_eq!(content_kind("readme.txt"), Some(ContentKind::PlainText));
        assert_eq!(content_kind("report.pdf"), Some(ContentKind::Pdf));
        assert_eq!(content_kind("proposal.docx"), Some(ContentKind::Docx));
        assert_eq!(content_kind("budget.xlsx"), Some(ContentKind::Xlsx));
        assert_eq!(content_kind("pitch.pptx"), Some(ContentKind::Pptx));
        assert_eq!(content_kind("photo.png"), None);
    }

    #[test]
    fn extracts_docx_paragraphs_tables_and_entities() {
        let xml = br#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Hello &amp; world</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Alpha</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Beta</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let text = normalize_extracted_text(extract_wordprocessing_xml(xml).unwrap());
        assert!(text.contains("Hello & world"), "extracted text: {text:?}");
        assert!(text.contains("Alpha"));
        assert!(text.contains("Beta"));
    }

    #[test]
    fn extracts_real_docx_package() {
        let document = br#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>DOCX body text</w:t></w:r></w:p></w:body></w:document>"#;
        let path = write_office_package("docx", &[("word/document.xml", document)]);
        let extraction = extract_file_content(&path, "document.docx");
        assert_eq!(extraction.status, ExtractionStatus::Extracted);
        assert!(extraction.text.contains("DOCX body text"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn extracts_pptx_text_in_paragraph_order() {
        let xml = br#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><a:p><a:r><a:t>Quarterly</a:t></a:r></a:p><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></p:cSld></p:sld>"#;
        let text = normalize_extracted_text(extract_presentation_xml(xml).unwrap());
        assert_eq!(text, "Quarterly\nRevenue");
    }

    #[test]
    fn extracts_real_pptx_package_with_slide_positions() {
        let slide = br#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><a:p><a:r><a:t>PPTX body text</a:t></a:r></a:p></p:cSld></p:sld>"#;
        let path = write_office_package("pptx", &[("ppt/slides/slide2.xml", slide)]);
        let extraction = extract_file_content(&path, "presentation.pptx");
        assert_eq!(extraction.status, ExtractionStatus::Extracted);
        assert!(extraction.text.contains("[Slide 2]"));
        assert!(extraction.text.contains("PPTX body text"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn extracts_xlsx_shared_inline_numeric_and_boolean_cells() {
        let shared_xml =
            br#"<sst><si><t>Name</t></si><si><r><t>North</t></r><r><t> Region</t></r></si></sst>"#;
        let shared = parse_shared_strings(shared_xml).unwrap();
        let sheet_xml = br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>Value</t></is></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>42</v></c><c r="C2" t="b"><v>1</v></c></row></sheetData></worksheet>"#;
        let text = normalize_extracted_text(extract_worksheet_xml(sheet_xml, &shared).unwrap());
        assert_eq!(
            text,
            "A1: Name\tB1: Value\nA2: North Region\tB2: 42\tC2: true"
        );
    }

    #[test]
    fn extracts_real_xlsx_package_with_sheet_and_cell_positions() {
        let workbook = br#"<workbook xmlns:r="r"><sheets><sheet name="Summary" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let relationships = br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let strings = br#"<sst><si><t>XLSX body text</t></si></sst>"#;
        let sheet = br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>88</v></c></row></sheetData></worksheet>"#;
        let path = write_office_package(
            "xlsx",
            &[
                ("xl/workbook.xml", workbook),
                ("xl/_rels/workbook.xml.rels", relationships),
                ("xl/sharedStrings.xml", strings),
                ("xl/worksheets/sheet1.xml", sheet),
            ],
        );
        let extraction = extract_file_content(&path, "workbook.xlsx");
        assert_eq!(extraction.status, ExtractionStatus::Extracted);
        assert!(extraction.text.contains("[Sheet: Summary]"));
        assert!(extraction.text.contains("A1: XLSX body text"));
        assert!(extraction.text.contains("B1: 88"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn extracts_real_pdf_with_page_position() {
        let path = temporary_path("pdf");
        fs::write(&path, simple_pdf_bytes("PDF body text")).unwrap();
        let extraction = extract_file_content(&path, "report.pdf");
        assert_eq!(extraction.status, ExtractionStatus::Extracted);
        assert!(extraction.text.contains("[Page 1]"));
        assert!(extraction.text.contains("PDF body text"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn extracts_pdf_text_when_non_text_graphics_state_is_missing() {
        let path = temporary_path("pdf");
        fs::write(
            &path,
            simple_pdf_bytes_with_content(
                "q /MissingGraphicsState gs BT /F1 12 Tf 72 720 Td (PDF text survives) Tj ET Q",
            ),
        )
        .unwrap();
        let extraction = extract_file_content(&path, "report.pdf");
        assert_eq!(extraction.status, ExtractionStatus::Extracted);
        assert!(extraction.text.contains("PDF text survives"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn unsupported_files_never_fabricate_body_text() {
        let extraction = extract_file_content(Path::new("not-read.png"), "not-read.png");
        assert_eq!(extraction.status, ExtractionStatus::Unsupported);
        assert!(extraction.text.is_empty());
        assert!(extraction.error.is_none());
    }
}
