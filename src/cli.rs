use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, Utc};
use clap_derive::{Parser, Subcommand};

use crate::{
    execute, list_session_names, read_script_from_files, read_script_from_stdin, read_session,
    remove_session, write_session, CommandRecord, CommandStatus, Environment, Session,
    SessionSummary,
};

#[derive(Debug, Parser)]
pub struct RunAction {
    #[arg(short, long)]
    interactive: bool,
    #[arg(short, long)]
    unchecked: bool,
    #[arg(short, long, conflicts_with_all = ["session", "command"])]
    file: Vec<PathBuf>,
    #[arg(short, long, conflicts_with_all = ["file", "command"])]
    session: Vec<String>,
    #[arg(conflicts_with_all = ["file", "session"])]
    command: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ShowAction {
    #[arg(short, long)]
    script: bool,
    session: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ListAction {
    #[arg(short, long)]
    full: bool,
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

fn resolve_target(target: &str, session_names: &[String]) -> Result<String> {
    if let Some(index) = parse_index(target) {
        let name = session_names.get(index).context("index out of range")?;
        Ok(name.clone())
    } else {
        if !session_names.iter().any(|name| name == target) {
            bail!("could not find session {}", target);
        }
        Ok(target.to_owned())
    }
}

fn resolve_targets<I: Iterator<Item = S>, S: AsRef<str>>(
    targets: I,
    session_names: &[String],
) -> Result<Vec<String>> {
    targets
        .map(|target| {
            resolve_target(target.as_ref(), session_names)
                .with_context(|| format!("could not resolve target {}", target.as_ref()))
        })
        .collect()
}

fn collect_commands(sessions: &[SessionSummary]) -> Vec<String> {
    sessions.iter().flat_map(|session| session.records.iter().map(|r| r.command.clone())).collect()
}

fn lookup_commands<I: Iterator<Item = S>, S: AsRef<str>>(
    targets: I,
    session_names: &[String],
) -> Result<Vec<String>> {
    let resolved = resolve_targets(targets, session_names).context("could not resolve targets")?;
    let sessions = resolved
        .into_iter()
        .map(|name| {
            read_session(&name)
                .map(|session| session.summary())
                .with_context(|| format!("could not read session {}", name))
        })
        .collect::<Result<Vec<SessionSummary>>>()?;
    Ok(collect_commands(&sessions))
}

fn run_command(env: Environment, command: String) -> Result<(Environment, CommandRecord, bool)> {
    println!("$ {}", command);

    let result = execute(&command, env, &mut std::io::stdout().lock())
        .with_context(|| format!("could not execute command {}", command))?;

    if needs_newline(&result.output) {
        println!();
    }

    let status = match result.succeeded {
        true => CommandStatus::Succeeded,
        false => CommandStatus::Failed,
    };
    let record = CommandRecord { command, output: result.output, status };
    let ok = record.status.is_succeeded();

    Ok((result.new_env, record, ok))
}

pub fn run(action: RunAction) -> Result<()> {
    let RunAction {
        interactive,
        unchecked,
        file: file_args,
        session: session_args,
        command: command_args,
    } = action;

    let checked = !unchecked;
    let from_file = !file_args.is_empty();
    let from_session = !session_args.is_empty();
    let from_command = !command_args.is_empty();

    let commands = if from_file {
        read_script_from_files(file_args.iter()).context("could not read script from file")?
    } else if from_session {
        let session_names = list_session_names().context("could not list sessions")?;
        lookup_commands(session_args.iter(), &session_names).context("could not lookup commands")?
    } else if from_command {
        command_args
    } else if !interactive {
        read_script_from_stdin().context("could not read script from STDIN")?
    } else {
        Vec::new()
    };

    let mut terminated = false;
    let mut env = Environment::default();
    let mut records = Vec::new();

    for command in commands.into_iter() {
        if terminated {
            records.push(CommandRecord {
                command,
                output: Default::default(),
                status: CommandStatus::Skipped,
            });
            continue;
        }
        if !records.is_empty() {
            println!();
        }
        let (e, r, ok) = run_command(env, command)?;
        env = e;
        records.push(r);
        terminated = terminated || (checked && !ok);
    }

    if interactive {
        let mut lines = std::io::stdin().lines();
        loop {
            if terminated {
                break;
            }
            if !records.is_empty() {
                println!();
            }

            eprint!("==> ");
            let command = match lines.next() {
                Some(c) => c.context("could not read next command from STDIN")?,
                None => break,
            };

            let (e, r, ok) = run_command(env, command)?;
            env = e;
            records.push(r);
            terminated = terminated || (checked && !ok);
        }
    }

    let session = Session::new(Utc::now(), records);
    write_session(&session).context("could not write session data")?;
    eprintln!("\nsession {} recorded", session.name);

    if terminated {
        bail!("command terminated with non-zero exit code");
    }
    Ok(())
}

pub fn show(action: ShowAction) -> Result<()> {
    let ShowAction { script, session: target_args } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let targets: Vec<String> = match target_args.is_empty() && !session_names.is_empty() {
        true => vec![session_names[0].clone()],
        false => resolve_targets(target_args.iter(), &session_names)
            .context("invalid `--session` argument")?,
    };

    for target in targets {
        let session = read_session(&target).context("could not read session data")?;
        eprintln!("session {} ({})", session.name, format_datetime(session.recorded_at));

        for (i, record) in session.records.into_iter().enumerate() {
            if script {
                println!("{}", record.command);
                continue;
            }

            if !record.status.is_executed() {
                continue;
            }
            if i > 0 {
                println!();
            }
            println!("$ {}", record.command);

            print!("{}", record.output);
            if needs_newline(&record.output) {
                println!();
            }
        }
    }

    Ok(())
}

pub fn list(action: ListAction) -> Result<()> {
    let ListAction { full, limit } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let limit = limit.min(session_names.len());

    for (index, target) in session_names[0..limit].iter().enumerate() {
        let session = read_session(target).context("could not read session data")?;
        println!("{}: {} ({})", index + 1, session.name, format_datetime(session.recorded_at));
        let len = session.records.len();
        let n = if full { len } else { 5.min(len) };
        let rem = len - n;
        for record in session.records.iter().take(n) {
            let marker = match record.status {
                CommandStatus::Succeeded => "$",
                CommandStatus::Failed => "$",
                CommandStatus::Skipped => "?",
            };
            println!("    {} {}", marker, record.command);
        }
        if rem > 0 {
            println!("    ... ({} more commands)", rem);
        }
        println!();
    }

    println!("({} / {} sessions)", limit, session_names.len());

    Ok(())
}

pub fn remove(action: RemoveAction) -> Result<()> {
    let RemoveAction { all, session: target_args } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let targets: Vec<String> = match all {
        true => session_names,
        false => resolve_targets(target_args.iter(), &session_names)
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

    use crate::{CommandRecordSummary, SessionSummary};

    use super::*;

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
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_target("@2", &names);
        let expected = Some("test2".into());
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_resolve_target_by_name() {
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_target("test1", &names);
        let expected = Some("test1".into());
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_resolve_target_index_out_of_range() {
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_target("@123", &names);
        assert!(actual.is_err());
    }

    #[test]
    fn test_resolve_target_unknown_name() {
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_target("test3", &names);
        assert!(actual.is_err());
    }

    #[test]
    fn test_collect_commands() {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().into();
        let sessions = vec![
            SessionSummary {
                name: "test1".into(),
                recorded_at: now,
                records: vec![
                    CommandRecordSummary {
                        command: "cmd1a".into(),
                        status: CommandStatus::Succeeded,
                    },
                    CommandRecordSummary {
                        command: "cmd1b".into(),
                        status: CommandStatus::Succeeded,
                    },
                ],
            },
            SessionSummary {
                name: "test2".into(),
                recorded_at: now,
                records: vec![
                    CommandRecordSummary {
                        command: "cmd2a".into(),
                        status: CommandStatus::Succeeded,
                    },
                    CommandRecordSummary {
                        command: "cmd2b".into(),
                        status: CommandStatus::Succeeded,
                    },
                    CommandRecordSummary {
                        command: "cmd2c".into(),
                        status: CommandStatus::Succeeded,
                    },
                ],
            },
        ];
        let actual = collect_commands(&sessions);
        let expected: Vec<String> = vec!["cmd1a", "cmd1b", "cmd2a", "cmd2b", "cmd2c"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(expected, actual);
    }
}
