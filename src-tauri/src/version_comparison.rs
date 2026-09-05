//! Local, read-only comparison of recorded text snapshots. No model/network calls.
//! Paths always come from the pinned workspace database, never from an AI response.
use crate::database::Database;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::Manager;

const MAX_VERSIONS: usize = 32;
const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIFF_CHARACTERS: usize = 24_000;
const MAX_LCS_CELLS: usize = 1_000_000;
const MAX_LINES: usize = 40_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileTarget {
    pub file_id: String,
    pub file_name: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", content = "numbers", rename_all = "snake_case")]
pub(crate) enum VersionSelection {
    All,
    LatestPair,
    Numbers(Vec<i64>),
    Range(Vec<i64>),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ComparisonError {
    Cancelled,
    Unavailable,
    MissingVersions,
    NotText,
    TooLarge,
    TooManyVersions,
    TooComplex,
    TooMuchDiff,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SnapshotVersion {
    pub id: String,
    pub number: i64,
    #[serde(skip)]
    path: String,
    #[serde(skip)]
    sha256: String,
    #[serde(skip)]
    size: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct VersionDiff {
    pub before: SnapshotVersion,
    pub after: SnapshotVersion,
    pub diff: String,
}

fn active(cancelled: &AtomicBool) -> Result<(), ComparisonError> {
    if cancelled.load(Ordering::Acquire) {
        Err(ComparisonError::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn resolve_file_target(
    database: &Database,
    id: &str,
) -> Result<Option<FileTarget>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Database unavailable".to_owned())?;
    connection
        .query_row(
            "SELECT id, original_name, COALESCE(storage_path, original_name) FROM files
         WHERE id = ?1 AND trashed_at IS NULL",
            [id],
            |row| {
                Ok(FileTarget {
                    file_id: row.get(0)?,
                    file_name: row.get(1)?,
                    relative_path: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn load_versions(
    database: &Database,
    file_id: &str,
    selection: &VersionSelection,
) -> Result<Vec<SnapshotVersion>, ComparisonError> {
    let (minimum, maximum, wanted, descending) = match selection {
        VersionSelection::All => (1, i64::MAX, Vec::new(), false),
        VersionSelection::LatestPair => (1, i64::MAX, Vec::new(), true),
        VersionSelection::Numbers(numbers) => {
            let mut values = numbers.clone();
            values.sort_unstable();
            values.dedup();
            if values.len() < 2 || values.len() > MAX_VERSIONS || values[0] <= 0 {
                return Err(ComparisonError::MissingVersions);
            }
            (values[0], *values.last().unwrap(), values, false)
        }
        VersionSelection::Range(numbers) => {
            if numbers.len() != 2 || numbers[0] <= 0 || numbers[1] <= numbers[0] {
                return Err(ComparisonError::MissingVersions);
            }
            if numbers[1] - numbers[0] >= MAX_VERSIONS as i64 {
                return Err(ComparisonError::TooManyVersions);
            }
            (
                numbers[0],
                numbers[1],
                (numbers[0]..=numbers[1]).collect(),
                false,
            )
        }
    };
    let wanted_json = serde_json::to_string(&wanted).map_err(|_| ComparisonError::Unavailable)?;
    let connection = database
        .0
        .lock()
        .map_err(|_| ComparisonError::Unavailable)?;
    // Range/explicit selections must not fetch unrelated history, even for very old versions.
    let sql = format!(
        "SELECT v.id, v.version_number, v.snapshot_path, v.sha256, v.size_bytes
         FROM file_space_artifact_versions v JOIN files f ON f.id = v.file_id
         WHERE v.file_id = ?1 AND f.trashed_at IS NULL AND v.version_number BETWEEN ?2 AND ?3
         AND (?4 = '[]' OR v.version_number IN (SELECT value FROM json_each(?4)))
         ORDER BY v.version_number {} LIMIT ?5",
        if descending { "DESC" } else { "ASC" }
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|_| ComparisonError::Unavailable)?;
    let mut versions = statement
        .query_map(
            params![
                file_id,
                minimum,
                maximum,
                wanted_json,
                if descending { 2 } else { MAX_VERSIONS + 1 }
            ],
            |row| {
                Ok(SnapshotVersion {
                    id: row.get(0)?,
                    number: row.get(1)?,
                    path: row.get(2)?,
                    sha256: row.get(3)?,
                    size: row.get(4)?,
                })
            },
        )
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|_| ComparisonError::Unavailable)?;
    if versions.len() > MAX_VERSIONS {
        return Err(ComparisonError::TooManyVersions);
    }
    versions.sort_by_key(|version| version.number);
    if versions.len() < 2
        || (!wanted.is_empty() && versions.iter().map(|v| v.number).collect::<Vec<_>>() != wanted)
    {
        return Err(ComparisonError::MissingVersions);
    }
    let total = versions.iter().try_fold(0usize, |total, version| {
        let size = usize::try_from(version.size).map_err(|_| ComparisonError::Unavailable)?;
        if size > MAX_SNAPSHOT_BYTES {
            return Err(ComparisonError::TooLarge);
        }
        total.checked_add(size).ok_or(ComparisonError::TooLarge)
    })?;
    if total > MAX_TOTAL_BYTES {
        return Err(ComparisonError::TooLarge);
    }
    Ok(versions)
}

fn read_text(version: &SnapshotVersion, cancelled: &AtomicBool) -> Result<String, ComparisonError> {
    active(cancelled)?;
    let metadata = fs::symlink_metadata(&version.path).map_err(|_| ComparisonError::Unavailable)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != version.size as u64
    {
        return Err(ComparisonError::Unavailable);
    }
    let mut file = fs::File::open(&version.path).map_err(|_| ComparisonError::Unavailable)?;
    let mut bytes = Vec::with_capacity(version.size as usize);
    let mut buffer = [0u8; 16 * 1024];
    loop {
        active(cancelled)?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| ComparisonError::Unavailable)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_SNAPSHOT_BYTES || bytes.len() + count > version.size as usize {
            return Err(ComparisonError::Unavailable);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() != version.size as usize
        || format!("{:x}", Sha256::digest(&bytes)) != version.sha256
    {
        return Err(ComparisonError::Unavailable);
    }
    let text = String::from_utf8(bytes).map_err(|_| ComparisonError::NotText)?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ComparisonError::NotText);
    }
    Ok(text)
}

// Bounded LCS, after stripping unchanged prefix/suffix. Large unrelated rewrites
// fail explicitly instead of allocating quadratic memory or silently truncating.
fn text_diff(before: &str, after: &str, cancelled: &AtomicBool) -> Result<String, ComparisonError> {
    active(cancelled)?;
    if before == after {
        return Ok("No content changes.".to_owned());
    }
    let old = before
        .split_inclusive('\n')
        .take(MAX_LINES + 1)
        .collect::<Vec<_>>();
    let new = after
        .split_inclusive('\n')
        .take(MAX_LINES + 1)
        .collect::<Vec<_>>();
    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        return Err(ComparisonError::TooComplex);
    }
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let width = b.len() + 1;
    let cells = (a.len() + 1)
        .checked_mul(width)
        .ok_or(ComparisonError::TooComplex)?;
    if cells > MAX_LCS_CELLS {
        return Err(ComparisonError::TooComplex);
    }
    let mut lcs = vec![0u32; cells];
    for i in (0..a.len()).rev() {
        active(cancelled)?;
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut operations = Vec::new();
    operations.extend(old[..prefix].iter().map(|line| (' ', *line)));
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            operations.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if i < a.len()
            && (j == b.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
        {
            operations.push(('-', a[i]));
            i += 1;
        } else {
            operations.push(('+', b[j]));
            j += 1;
        }
    }
    operations.extend(old[old.len() - suffix..].iter().map(|line| (' ', *line)));
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (index, _) in operations
        .iter()
        .enumerate()
        .filter(|(_, (tag, _))| *tag != ' ')
    {
        let start = index.saturating_sub(3);
        let end = (index + 4).min(operations.len());
        if let Some(last) = spans.last_mut().filter(|last| last.1 >= start) {
            last.1 = end;
        } else {
            spans.push((start, end));
        }
    }
    let mut output = String::new();
    let (mut old_line, mut new_line, mut cursor, mut characters) = (1, 1, 0, 0);
    for (start, end) in spans {
        active(cancelled)?;
        for (tag, _) in &operations[cursor..start] {
            old_line += usize::from(*tag != '+');
            new_line += usize::from(*tag != '-');
        }
        let old_count = operations[start..end]
            .iter()
            .filter(|(tag, _)| *tag != '+')
            .count();
        let new_count = operations[start..end]
            .iter()
            .filter(|(tag, _)| *tag != '-')
            .count();
        let old_start = old_line - usize::from(old_count == 0);
        let new_start = new_line - usize::from(new_count == 0);
        let header = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n");
        characters += header.chars().count();
        output.push_str(&header);
        for (tag, line) in &operations[start..end] {
            characters += 1 + line.chars().count() + if line.ends_with('\n') { 0 } else { 29 };
            if characters > MAX_DIFF_CHARACTERS {
                return Err(ComparisonError::TooMuchDiff);
            }
            output.push(*tag);
            output.push_str(line);
            if !line.ends_with('\n') {
                output.push_str("\n\\ No newline at end of file\n");
            }
        }
        old_line += old_count;
        new_line += new_count;
        cursor = end;
    }
    Ok(output)
}

pub(crate) fn compare_versions(
    database: &Database,
    file_id: &str,
    selection: &VersionSelection,
    cancelled: &AtomicBool,
) -> Result<Vec<VersionDiff>, ComparisonError> {
    active(cancelled)?;
    let target = resolve_file_target(database, file_id)
        .map_err(|_| ComparisonError::Unavailable)?
        .ok_or(ComparisonError::Unavailable)?;
    let extension = Path::new(&target.file_name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    if matches!(
        extension.as_str(),
        "pdf" | "docx" | "xlsx" | "pptx" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "zip"
    ) {
        return Err(ComparisonError::NotText);
    }
    let versions = load_versions(database, file_id, selection)?;
    let mut previous = read_text(&versions[0], cancelled)?;
    let mut result = Vec::new();
    let mut total = 0;
    for pair in versions.windows(2) {
        let current = read_text(&pair[1], cancelled)?;
        let diff = text_diff(&previous, &current, cancelled)?;
        total += diff.chars().count();
        if total > MAX_DIFF_CHARACTERS {
            return Err(ComparisonError::TooMuchDiff);
        }
        result.push(VersionDiff {
            before: pair[0].clone(),
            after: pair[1].clone(),
            diff,
        });
        previous = current;
    }
    active(cancelled)?;
    Ok(result)
}

/// Local-only backend operation. It neither writes chat history nor constructs
/// an AI prompt. Registering it does not start comparisons or index any history.
#[tauri::command]
pub(crate) async fn get_task_file_text_diff(
    app: tauri::AppHandle,
    file_id: String,
    selection: VersionSelection,
) -> Result<Vec<VersionDiff>, ComparisonError> {
    tauri::async_runtime::spawn_blocking(move || {
        let database = {
            let _operation = crate::file_space::lock_file_space_operations()
                .map_err(|_| ComparisonError::Unavailable)?;
            let location = app
                .state::<Database>()
                .location()
                .map_err(|_| ComparisonError::Unavailable)?;
            crate::database::open_workspace_database(
                location.database_path,
                location.artifact_store_path,
                location.workspace_id,
            )
            .map_err(|_| ComparisonError::Unavailable)?
        };
        compare_versions(&database, &file_id, &selection, &AtomicBool::new(false))
    })
    .await
    .map_err(|_| ComparisonError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Fixture {
        directory: std::path::PathBuf,
        database: Database,
    }
    impl Fixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("lumetrace-synthetic-diff-{}", Uuid::new_v4()));
            fs::create_dir(&directory).unwrap();
            let database = crate::database::open_database(directory.join("test.sqlite3")).unwrap();
            {
                let connection = database.0.lock().unwrap();
                connection.execute("INSERT INTO files (id, original_name, storage_path, created_at) VALUES ('synthetic', 'synthetic.txt', 'synthetic.txt', 0)", []).unwrap();
                connection.execute("INSERT INTO file_space_artifacts (file_id, task_id, logical_key, created_at, updated_at) VALUES ('synthetic', 'fixture', 'synthetic', 0, 0)", []).unwrap();
                for (index, content) in ["version one", "version two", "version three"]
                    .iter()
                    .enumerate()
                {
                    let number = index as i64 + 1;
                    let path = directory.join(format!("v{number}.txt"));
                    fs::write(&path, content).unwrap();
                    connection.execute("INSERT INTO file_space_artifact_versions (id, file_id, version_number, snapshot_path, sha256, size_bytes, produced_name, origin, produced_at, created_at) VALUES (?1, 'synthetic', ?2, ?3, ?4, ?5, 'synthetic.txt', 'user_edit', 0, 0)", params![format!("v{number}"), number, path.to_string_lossy(), format!("{:x}", Sha256::digest(content.as_bytes())), content.len()]).unwrap();
                }
            }
            Self {
                directory,
                database,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn three_synthetic_versions_are_compared_locally() {
        let fixture = Fixture::new();
        let result = compare_versions(
            &fixture.database,
            "synthetic",
            &VersionSelection::All,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!((result[0].before.number, result[0].after.number), (1, 2));
        assert!(result[0].diff.contains("-version one\n"));
        assert!(result[0].diff.contains("+version two\n"));
        assert!(result[1].diff.contains("+version three\n"));
        assert_eq!(result[0].before.id, "v1");
        let result = compare_versions(
            &fixture.database,
            "synthetic",
            &VersionSelection::Numbers(vec![1, 3]),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].diff.contains("version two"));
    }

    #[test]
    fn missing_deleted_wrong_workspace_corruption_and_cancellation_never_fall_back() {
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::Numbers(vec![1, 99]),
                &cancel
            )
            .unwrap_err(),
            ComparisonError::MissingVersions
        );
        assert_eq!(
            compare_versions(
                &fixture.database,
                "other-workspace-id",
                &VersionSelection::All,
                &cancel
            )
            .unwrap_err(),
            ComparisonError::Unavailable
        );
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::All,
                &AtomicBool::new(true)
            )
            .unwrap_err(),
            ComparisonError::Cancelled
        );
        fs::write(fixture.directory.join("v1.txt"), "version xxx").unwrap();
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::All,
                &cancel
            )
            .unwrap_err(),
            ComparisonError::Unavailable
        );
        fixture
            .database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at = 1 WHERE id = 'synthetic'", [])
            .unwrap();
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::LatestPair,
                &cancel
            )
            .unwrap_err(),
            ComparisonError::Unavailable
        );
    }

    #[test]
    fn explicit_selection_does_not_read_other_snapshots_and_latest_is_ordered() {
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        let latest = compare_versions(
            &fixture.database,
            "synthetic",
            &VersionSelection::LatestPair,
            &cancel,
        )
        .unwrap();
        assert_eq!((latest[0].before.number, latest[0].after.number), (2, 3));
        fs::write(fixture.directory.join("v2.txt"), "corrupted unused version").unwrap();
        let selected = compare_versions(
            &fixture.database,
            "synthetic",
            &VersionSelection::Numbers(vec![3, 1, 1]),
            &cancel,
        )
        .unwrap();
        assert_eq!(
            (selected[0].before.number, selected[0].after.number),
            (1, 3)
        );
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::Range(vec![1, 3]),
                &cancel
            )
            .unwrap_err(),
            ComparisonError::Unavailable
        );
        let encoded = serde_json::to_value(&selected).unwrap();
        assert_eq!(
            encoded[0]["before"],
            serde_json::json!({"id": "v1", "number": 1})
        );
        assert!(!encoded.to_string().contains("snapshot_path"));
        assert!(!encoded
            .to_string()
            .contains(&fixture.directory.to_string_lossy().to_string()));
    }

    #[test]
    fn resource_limits_reject_before_reading_files() {
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        fixture
            .database
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE file_space_artifact_versions SET size_bytes = ?1 WHERE id = 'v1'",
                [MAX_SNAPSHOT_BYTES + 1],
            )
            .unwrap();
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::All,
                &cancel
            )
            .unwrap_err(),
            ComparisonError::TooLarge
        );
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::Range(vec![1, 99]),
                &cancel
            )
            .unwrap_err(),
            ComparisonError::TooManyVersions
        );
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::Numbers(vec![0, 1]),
                &cancel
            )
            .unwrap_err(),
            ComparisonError::MissingVersions
        );
        fixture.database.0.lock().unwrap().execute_batch(
            "WITH RECURSIVE numbers(n) AS (SELECT 4 UNION ALL SELECT n + 1 FROM numbers WHERE n < 33)
             INSERT INTO file_space_artifact_versions (id, file_id, version_number, snapshot_path, sha256, size_bytes, produced_name, origin, produced_at, created_at)
             SELECT 'extra-' || n, 'synthetic', n, 'not-read', 'not-read', 1, 'synthetic.txt', 'user_edit', 0, 0 FROM numbers;"
        ).unwrap();
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::All,
                &cancel
            )
            .unwrap_err(),
            ComparisonError::TooManyVersions
        );
    }

    #[test]
    fn identical_file_ids_in_another_workspace_do_not_share_snapshots() {
        let first = Fixture::new();
        let second = Fixture::new();
        fs::write(first.directory.join("v1.txt"), "corrupt workspace one").unwrap();
        assert!(compare_versions(
            &second.database,
            "synthetic",
            &VersionSelection::All,
            &AtomicBool::new(false)
        )
        .is_ok());
        assert_eq!(
            compare_versions(
                &first.database,
                "synthetic",
                &VersionSelection::All,
                &AtomicBool::new(false)
            )
            .unwrap_err(),
            ComparisonError::Unavailable
        );
    }

    #[test]
    fn binary_snapshots_are_rejected_without_lossy_text_conversion() {
        let fixture = Fixture::new();
        let content = [0xffu8, 0x00];
        fs::write(fixture.directory.join("v1.txt"), content).unwrap();
        fixture.database.0.lock().unwrap().execute("UPDATE file_space_artifact_versions SET size_bytes = 2, sha256 = ?1 WHERE id = 'v1'", [format!("{:x}", Sha256::digest(content))]).unwrap();
        assert_eq!(
            compare_versions(
                &fixture.database,
                "synthetic",
                &VersionSelection::All,
                &AtomicBool::new(false)
            )
            .unwrap_err(),
            ComparisonError::NotText
        );
    }

    #[test]
    fn diff_is_bounded_and_preserves_multiple_hunks_and_newlines() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            text_diff("same", "same", &cancel).unwrap(),
            "No content changes."
        );
        let diff = text_diff(
            "old\na\nb\nc\nd\ne\nf\ng\nh\ni\nend",
            "new\na\nb\nc\nd\ne\nf\ng\nh\ni\nlast\n",
            &cancel,
        )
        .unwrap();
        assert_eq!(diff.matches("@@ -").count(), 2);
        assert!(diff.contains("-end\n\\ No newline at end of file"));
        assert!(diff.contains("+last\n"));
        assert_eq!(
            text_diff(&"a\n".repeat(1100), &"b\n".repeat(1100), &cancel),
            Err(ComparisonError::TooComplex)
        );
        assert_eq!(
            text_diff("old", &"x".repeat(MAX_DIFF_CHARACTERS), &cancel),
            Err(ComparisonError::TooMuchDiff)
        );
        assert!(text_diff("", "新增\n", &cancel)
            .unwrap()
            .starts_with("@@ -0,0 +1,1 @@"));
        assert!(text_diff("移除\n", "", &cancel)
            .unwrap()
            .starts_with("@@ -1,1 +0,0 @@"));
        assert_eq!(
            text_diff(&"a\n".repeat(MAX_LINES + 1), "", &cancel),
            Err(ComparisonError::TooComplex)
        );
    }
}
