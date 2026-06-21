use serde::{de::DeserializeOwned, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::error::Result;
use crate::lock::FileLock;

pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    append_jsonl_locked(&path.with_extension("lock"), path, value)
}

pub fn append_jsonl_locked<T: Serialize>(lock_path: &Path, path: &Path, value: &T) -> Result<()> {
    let _lock = FileLock::exclusive(lock_path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = OpenOptions::new().read(true).open(path)?;
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .collect::<std::io::Result<_>>()?;
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(value) => out.push(value),
            Err(error) => {
                // Tolerate a torn final line: a crash mid-append can leave a
                // partial JSON record with no trailing newline. Only the last
                // non-empty line may be skipped; corruption before it is fatal.
                let rest_is_blank = lines[index + 1..].iter().all(|l| l.trim().is_empty());
                if rest_is_blank {
                    break;
                }
                return Err(error.into());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Row {
        value: String,
    }

    #[test]
    fn append_and_read_jsonl_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        append_jsonl(&path, &Row { value: "a".into() }).unwrap();
        append_jsonl(&path, &Row { value: "b".into() }).unwrap();
        let rows: Vec<Row> = read_jsonl(&path).unwrap();
        assert_eq!(
            rows,
            vec![Row { value: "a".into() }, Row { value: "b".into() }]
        );
    }

    #[test]
    fn read_jsonl_tolerates_torn_final_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        append_jsonl(&path, &Row { value: "a".into() }).unwrap();
        // Simulate a crash that left a partial record with no terminating newline.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"value\":\"b").unwrap();
        let rows: Vec<Row> = read_jsonl(&path).unwrap();
        assert_eq!(rows, vec![Row { value: "a".into() }]);
    }

    #[test]
    fn read_jsonl_rejects_corruption_before_final_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"not json\n{\"value\":\"b\"}\n").unwrap();
        let result: Result<Vec<Row>> = read_jsonl(&path);
        assert!(result.is_err());
    }

    #[test]
    fn concurrent_appends_do_not_interleave_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("rows.jsonl"));
        let mut handles = Vec::new();

        for worker in 0..8 {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                for seq in 0..50 {
                    append_jsonl(
                        path.as_path(),
                        &Row {
                            value: format!("{worker}:{seq}"),
                        },
                    )
                    .unwrap();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let rows: Vec<Row> = read_jsonl(path.as_path()).unwrap();
        assert_eq!(rows.len(), 400);
    }
}
