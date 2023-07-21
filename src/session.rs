use std::fs::{create_dir_all, remove_file, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub output: String,
    pub status: CommandStatus,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub recorded_at: DateTime<Utc>,
    pub records: Vec<CommandRecord>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandRecordSummary {
    pub command: String,
    pub status: CommandStatus,
}

#[derive(Debug, PartialEq)]
pub struct SessionSummary {
    pub name: String,
    pub recorded_at: DateTime<Utc>,
    pub records: Vec<CommandRecordSummary>,
}

fn generate_session_key(now: DateTime<Utc>) -> String {
    let now = now.format("%Y%m%d%H%M%S%3f");
    let charset = b"0123456789abcdef";
    let mut rng = rand::thread_rng();
    let suffix = (0..8).flat_map(|_| charset.choose(&mut rng).copied().into_iter()).collect();
    let suffix_string = String::from_utf8(suffix).expect("should not fail");
    format!("{}-{}", now, suffix_string)
}

impl CommandStatus {
    pub fn is_executed(&self) -> bool {
        match self {
            CommandStatus::Succeeded => true,
            CommandStatus::Failed => true,
            CommandStatus::Skipped => false,
        }
    }
    pub fn is_succeeded(&self) -> bool {
        match self {
            CommandStatus::Succeeded => true,
            CommandStatus::Failed => false,
            CommandStatus::Skipped => false,
        }
    }
}

impl Session {
    pub fn new(recorded_at: DateTime<Utc>, records: Vec<CommandRecord>) -> Self {
        Session { name: generate_session_key(recorded_at), recorded_at, records }
    }
    pub fn summary(&self) -> SessionSummary {
        let records = self
            .records
            .iter()
            .map(|r| CommandRecordSummary { command: r.command.clone(), status: r.status })
            .collect();
        SessionSummary { name: self.name.clone(), recorded_at: self.recorded_at, records }
    }
}

fn write_session_to_file(path: impl AsRef<Path>, session: &Session) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        create_dir_all(parent).context("could not create parent directory")?;
    }
    let file = File::create(path).context("could not create file")?;
    serde_json::to_writer(file, session).context("could not write to file")
}

fn read_session_from_file(path: impl AsRef<Path>) -> Result<Session> {
    let path = path.as_ref();
    let file = File::open(path).context("could not open file")?;
    serde_json::from_reader(file).context("could not parse file")
}

fn list_sessions_from_dir(dir: impl AsRef<Path>) -> Result<Vec<SessionSummary>> {
    let dir = dir.as_ref();

    let mut sessions = Vec::new();

    for entry in dir.read_dir().context("could not read directory")? {
        let entry = match entry {
            Ok(entry) => entry,
            _ => continue,
        };
        let is_file = entry.file_type().map_or(false, |typ| typ.is_file());
        if !is_file {
            continue;
        }
        let fname = entry.file_name().into_string();
        let ends_with_json = fname.map(|fname| fname.ends_with(".json")).unwrap_or(false);
        if !ends_with_json {
            continue;
        }
        let path = entry.path();
        let session = read_session_from_file(&path)
            .with_context(|| format!("could not read session file at {}", path.display()))?;
        sessions.push(session.summary());
    }

    sessions.sort_by_cached_key(|summary| summary.name.clone());
    sessions.reverse();

    Ok(sessions)
}

fn get_session_dir() -> Result<PathBuf> {
    let base_dirs = xdg::BaseDirectories::with_prefix("scener")
        .context("could not locate xdg app data directory")?;
    Ok(base_dirs.get_data_file("sessions"))
}

pub fn write_session(session: &Session) -> Result<()> {
    let session_dir = get_session_dir().context("could not locate session data directory")?;
    let path = session_dir.join(format!("{}.json", session.name));
    write_session_to_file(&path, session)
        .with_context(|| format!("could not write session data into {}", path.display()))
}

pub fn read_session(name: &str) -> Result<Session> {
    let session_dir = get_session_dir().context("could not locate session data directory")?;
    let path = session_dir.join(format!("{}.json", name));
    read_session_from_file(&path)
        .with_context(|| format!("could not read session data from {}", path.display()))
}

pub fn list_sessions() -> Result<Vec<SessionSummary>> {
    let session_dir = get_session_dir().context("could not locate session data directory")?;
    list_sessions_from_dir(session_dir).context("could not list sessions in session directory")
}

pub fn remove_session(name: &str) -> Result<()> {
    let session_dir = get_session_dir().context("could not locate session data directory")?;
    let path = session_dir.join(format!("{}.json", name));
    remove_file(&path)
        .with_context(|| format!("could not remove session file at {}", path.display()))
}

#[cfg(test)]
mod test {

    use chrono::Duration;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_session_read_write() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let session = Session {
            name: "test".into(),
            recorded_at: now,
            records: vec![
                CommandRecord {
                    command: "cmd1".into(),
                    output: "out1".into(),
                    status: CommandStatus::Succeeded,
                },
                CommandRecord {
                    command: "cmd2".into(),
                    output: "out2".into(),
                    status: CommandStatus::Failed,
                },
            ],
        };

        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        assert!(write_session_to_file(temp_path.join("file"), &session).is_ok());

        let read = read_session_from_file(temp_path.join("file"));
        assert_eq!(Some(session), read.ok());
    }

    #[test]
    fn test_list_sessions_from_dir() {
        let now: DateTime<Utc> =
            DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let session1 = Session {
            name: "test1".into(),
            recorded_at: now.checked_add_signed(Duration::seconds(1)).unwrap(),
            records: vec![CommandRecord {
                command: "cmd1".into(),
                output: "out1".into(),
                status: CommandStatus::Succeeded,
            }],
        };
        let session2 = Session {
            name: "test2".into(),
            recorded_at: now.checked_add_signed(Duration::seconds(2)).unwrap(),
            records: vec![CommandRecord {
                command: "cmd2".into(),
                output: "out2".into(),
                status: CommandStatus::Failed,
            }],
        };
        let session3 = Session {
            name: "test3".into(),
            recorded_at: now.checked_add_signed(Duration::seconds(3)).unwrap(),
            records: vec![CommandRecord {
                command: "cmd3".into(),
                output: "out3".into(),
                status: CommandStatus::Failed,
            }],
        };

        // Should be sorted by `recoreded_at` in desc order.
        let expected = Some(vec![session3.summary(), session2.summary(), session1.summary()]);

        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        assert!(write_session_to_file(temp_path.join("a.json"), &session3).is_ok());
        assert!(write_session_to_file(temp_path.join("b.json"), &session1).is_ok());
        assert!(write_session_to_file(temp_path.join("c.json"), &session2).is_ok());

        let sessions = list_sessions_from_dir(temp_path);
        assert_eq!(expected, sessions.ok());
    }
}
