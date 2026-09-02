use crate::database::Database;
use rusqlite::OptionalExtension;
use std::{fs, path::Path};
use tauri::State;

const PDF_FILE_MAX_BYTES: u64 = 100 * 1024 * 1024;

fn require_storage_root(database: &Database) -> Result<std::path::PathBuf, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let path = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'file_space.storage_root'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the File Space storage path: {error}"))?
        .ok_or_else(|| "The File Space storage path is not configured".to_owned())?;
    let root = std::path::PathBuf::from(path);
    let metadata = fs::symlink_metadata(&root)
        .map_err(|error| format!("Unable to inspect the File Space storage path: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The File Space storage path is not a regular directory".to_owned());
    }
    root.canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))
}

fn pdf_record(database: &Database, file_id: &str) -> Result<(String, String), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT original_name, storage_path
             FROM files
             WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL",
            [file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Unable to load the PDF file: {error}"))?
        .ok_or_else(|| "The PDF file no longer exists".to_owned())
}

fn read_pdf_file(database: &Database, file_id: &str) -> Result<Vec<u8>, String> {
    let root = require_storage_root(database)?;
    let (name, relative_path) = pdf_record(database, file_id)?;
    let extension = Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() != Some("pdf") {
        return Err("Only PDF files can be opened in the PDF viewer".to_owned());
    }

    let candidate = root.join(&relative_path);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("Unable to inspect the PDF file: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The managed PDF must be a regular file".to_owned());
    }
    if metadata.len() > PDF_FILE_MAX_BYTES {
        return Err("The PDF file is larger than the 100 MB preview limit".to_owned());
    }
    let canonical_file = candidate
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the PDF file: {error}"))?;
    if !canonical_file.starts_with(&root) || canonical_file == root {
        return Err("The managed PDF is outside the File Space storage path".to_owned());
    }
    fs::read(&canonical_file).map_err(|error| format!("Unable to read the PDF file: {error}"))
}

#[tauri::command]
pub fn read_file_space_pdf(
    file_id: String,
    database: State<'_, Database>,
) -> Result<tauri::ipc::Response, String> {
    read_pdf_file(database.inner(), &file_id).map(tauri::ipc::Response::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use uuid::Uuid;

    #[test]
    fn reads_only_managed_pdf_files_within_the_size_limit() {
        let test_root = std::env::temp_dir().join(format!("lumetrace-pdf-{}", Uuid::new_v4()));
        let storage_root = test_root.join("storage");
        fs::create_dir_all(&storage_root).unwrap();
        let pdf_bytes = b"%PDF-1.4\nmanaged preview\n%%EOF\n";
        fs::write(storage_root.join("brief.pdf"), pdf_bytes).unwrap();
        fs::write(storage_root.join("notes.txt"), b"plain text").unwrap();
        fs::write(storage_root.join("large.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
        let database =
            crate::database::open_for_test(&test_root.join("lumetrace.sqlite3")).unwrap();
        {
            let connection = database.0.lock().unwrap();
            connection.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('file_space.storage_root', ?1, 1)",
                [storage_root.to_string_lossy().as_ref()],
            ).unwrap();
            for (id, name) in [
                ("pdf", "brief.pdf"),
                ("text", "notes.txt"),
                ("large", "large.pdf"),
            ] {
                connection.execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, mime_type, size_bytes, source_kind, created_at, updated_at)
                     VALUES (?1, ?2, ?2, NULL, 1, 'user_import', 1, 1)",
                    params![id, name],
                ).unwrap();
            }
        }
        fs::OpenOptions::new()
            .write(true)
            .open(storage_root.join("large.pdf"))
            .unwrap()
            .set_len(PDF_FILE_MAX_BYTES + 1)
            .unwrap();

        assert_eq!(read_pdf_file(&database, "pdf").unwrap(), pdf_bytes);
        assert!(read_pdf_file(&database, "text")
            .unwrap_err()
            .contains("Only PDF"));
        assert!(read_pdf_file(&database, "large")
            .unwrap_err()
            .contains("100 MB"));
        assert!(read_pdf_file(&database, "missing").is_err());
        fs::remove_dir_all(test_root).unwrap();
    }
}
