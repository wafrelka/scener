use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, Utc};
use clap_derive::{Parser, Subcommand};

use crate::{
    execute, list_sessions, read_script_from_files, read_script_from_stdin, read_session,
    remove_session, write_session, CommandRecord, Environment, Session, SessionSummary,
};

#[derive(Debug, Parser)]
pub struct RunAction {
    #[arg(short, long)]
    unchecked: bool,
    #[arg(short, long, conflicts_with = "session", conflicts_with = "command")]
    file: Vec<PathBuf>,
    #[arg(short, long, conflicts_with = "file", conflicts_with = "command")]
    session: Vec<String>,
    #[arg(conflicts_with = "file", conflicts_with = "session")]
    command: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ShowAction {
    #[arg(short, long)]
    summary: bool,
    session: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ListAction {
    #[arg(short, long)]
    summary: bool,
    #[arg(short, long, short_alias = 'n', default_value = "10")]
    limit: usize,
}

#[derive(Debug, Parser)]
pub struct RemoveAction {
    #[arg(long)]
    all: bool,
    session: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    Run(RunAction),
    Show(ShowAction),
    #[command(alias = "ls")]
    List(ListAction),
    #[command(alias = "rm")]
    Remove(RemoveAction),
}

#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub action: Action,
}

fn format_datetime(dt: DateTime<Utc>) -> String {
    let local: DateTime<Local> = dt.into();
    local.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn needs_newline(s: &str) -> bool {
    !s.is_empty() && !s.ends_with('\n')
}

fn parse_index(s: &str) -> Option<usize> {
    if s == "@" {
        return Some(0);
    }
    let i: usize = s.strip_prefix('@').and_then(|s| s.parse().ok())?;
    match i > 0 {
        true => Some(i - 1),
        false => None,
    }
}

fn resolve_target(target: &str, sessions: &[SessionSummary]) -> Result<String> {
    if let Some(index) = parse_index(target) {
        let session = sessions.get(index).context("index out of range")?;
        Ok(session.name.clone())
    } else {
        if !sessions.iter().any(|session| session.name == target) {
            bail!("could not find session {}", target);
        }
        Ok(target.to_owned())
    }
}

fn resolve_targets<I: Iterator<Item = S>, S: AsRef<str>>(
    targets: I,
    sessions: &[SessionSummary],
) -> Result<Vec<String>> {
    targets
        .map(|target| {
            resolve_target(target.as_ref(), sessions)
                .with_context(|| format!("could not resolve target {}", target.as_ref()))
        })
        .collect()
}

fn lookup_commands<I: Iterator<Item = S>, S: AsRef<str>>(
    targets: I,
    sessions: &[SessionSummary],
) -> Result<Vec<String>> {
    let resolved = resolve_targets(targets, sessions).context("could not resolve targets")?;
    Ok(resolved
        .into_iter()
        .map(|name| sessions.iter().find(|session| session.name == name))
        .flat_map(|session| session.unwrap().commands.clone())
        .collect())
}

pub fn run(action: RunAction) -> Result<()> {
    let RunAction { unchecked, file: file_args, session: session_args, command: command_args } =
        action;

    let has_file = !file_args.is_empty();
    let has_session = !session_args.is_empty();
    let has_command = !command_args.is_empty();

    let commands = match (has_file, has_session, has_command) {
        (true, _, _) => {
            read_script_from_files(file_args.iter()).context("could not read script from file")?
        }
        (_, true, _) => {
            let sessions = list_sessions().context("could not list sessions")?;
            lookup_commands(session_args.iter(), &sessions).context("could not lookup commands")?
        }
        (_, _, true) => command_args,
        _ => read_script_from_stdin().context("could not read script from STDIN")?,
    };

    let mut env = Environment::default();
    let mut records = Vec::new();

    for (i, command) in commands.into_iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("$ {}", command);

        let result = execute(&command, env, &mut std::io::stdout().lock())
            .with_context(|| format!("could not execute command {}", command))?;

        if needs_newline(&result.output) {
            println!();
        }

        if !unchecked && !result.succeeded {
            bail!("command terminated with non-zero exit code");
        }

        env = result.new_env;
        records.push(CommandRecord { command, output: result.output, succeeded: result.succeeded });
    }

    let session = Session::new(Utc::now(), records);
    write_session(&session).context("could not write session data")?;
    eprintln!("\nsession {} recorded", session.name);

    Ok(())
}

pub fn show(action: ShowAction) -> Result<()> {
    let ShowAction { summary, session: target_args } = action;

    let sessions = list_sessions().context("could not list sessions")?;
    let targets: Vec<String> = match target_args.is_empty() && !sessions.is_empty() {
        true => vec![sessions[0].name.clone()],
        false => resolve_targets(target_args.iter(), &sessions)
            .context("invalid `--session` argument")?,
    };

    for target in targets {
        let session = read_session(&target).context("could not read session data")?;
        eprintln!("session {} ({})", session.name, format_datetime(session.recorded_at));

        for (i, record) in session.records.into_iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("$ {}", record.command);

            if summary {
                continue;
            }

            print!("{}", record.output);
            if needs_newline(&record.output) {
                println!();
            }
        }
    }

    Ok(())
}

pub fn list(action: ListAction) -> Result<()> {
    let ListAction { summary, limit } = action;

    let sessions = list_sessions().context("could not list sessions")?;
    let limit = limit.min(sessions.len());
    println!("{} / {} sessions", limit, sessions.len());

    for (index, session) in sessions[0..limit].iter().enumerate() {
        println!("{}: {} ({})", index + 1, session.name, format_datetime(session.recorded_at));
        if summary {
            continue;
        }
        for command in &session.commands {
            println!("  $ {}", command);
        }
    }

    Ok(())
}

pub fn remove(action: RemoveAction) -> Result<()> {
    let RemoveAction { all, session: target_args } = action;

    let sessions = list_sessions().context("could not list sessions")?;
    let targets: Vec<String> = match all {
        true => sessions.iter().map(|session| session.name.clone()).collect(),
        false => resolve_targets(target_args.iter(), &sessions)
            .context("invalid `--session` argument")?,
    };

    for target in &targets {
        remove_session(target).context("could not remove session")?;
        println!("session {} removed", target);
    }

    Ok(())
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.action {
            Action::Run(action) => run(action),
            Action::Show(action) => show(action),
            Action::List(action) => list(action),
            Action::Remove(action) => remove(action),
        }
    }
}

#[cfg(test)]
mod test {
    use chrono::DateTime;

    use crate::cli::{lookup_commands, needs_newline, parse_index};
    use crate::SessionSummary;

    use super::resolve_target;

    #[test]
    fn test_needs_newline_empty() {
        assert!(!needs_newline(""));
    }

    #[test]
    fn test_needs_newline_with_newline() {
        assert!(!needs_newline("abc\ndef\n"));
    }

    #[test]
    fn test_needs_newline_without_newline() {
        assert!(needs_newline("abc\ndef"));
    }

    #[test]
    fn test_parse_index_bare_index() {
        assert_eq!(Some(0), parse_index("@"));
    }

    #[test]
    fn test_parse_index_zero_index() {
        assert_eq!(None, parse_index("@0"));
    }

    #[test]
    fn test_parse_index_one_index() {
        assert_eq!(Some(0), parse_index("@1"));
    }

    #[test]
    fn test_parse_index_integer_index() {
        assert_eq!(Some(4), parse_index("@5"));
    }

    #[test]
    fn test_parse_index_invalid_index() {
        assert_eq!(None, parse_index("@abc"));
    }

    #[test]
    fn test_resolve_target_by_index() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary { name: "test1".into(), recorded_at: now, commands: Vec::new() },
            SessionSummary { name: "test2".into(), recorded_at: now, commands: Vec::new() },
        ];
        let actual = resolve_target("@2", &sessions);
        let expected = Some("test2".into());
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_resolve_target_by_name() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary { name: "test1".into(), recorded_at: now, commands: Vec::new() },
            SessionSummary { name: "test2".into(), recorded_at: now, commands: Vec::new() },
        ];
        let actual = resolve_target("test1", &sessions);
        let expected = Some("test1".into());
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_resolve_target_index_out_of_range() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary { name: "test1".into(), recorded_at: now, commands: Vec::new() },
            SessionSummary { name: "test2".into(), recorded_at: now, commands: Vec::new() },
        ];
        let actual = resolve_target("@123", &sessions);
        assert!(actual.is_err());
    }

    #[test]
    fn test_resolve_target_unknown_name() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary { name: "test1".into(), recorded_at: now, commands: Vec::new() },
            SessionSummary { name: "test2".into(), recorded_at: now, commands: Vec::new() },
        ];
        let actual = resolve_target("test3", &sessions);
        assert!(actual.is_err());
    }

    #[test]
    fn test_lookup_commands() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary {
                name: "test1".into(),
                recorded_at: now,
                commands: vec!["cmd1a".into(), "cmd1b".into()],
            },
            SessionSummary {
                name: "test2".into(),
                recorded_at: now,
                commands: vec!["cmd2a".into(), "cmd2b".into(), "cmd2c".into()],
            },
        ];
        let actual = lookup_commands(vec!["test2", "test1"].iter(), &sessions);
        let expected: Vec<String> = vec!["cmd2a", "cmd2b", "cmd2c", "cmd1a", "cmd1b"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(Some(expected), actual.ok());
    }
}
