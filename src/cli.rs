use std::io::stderr;
use std::io::stdout;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

use crate::{
    execute, list_session_names, needs_newline, print_session, print_session_brief,
    print_session_script, read_script_from_files, read_script_from_stdin, read_session,
    remove_session, resolve_references, write_session, CommandRecord, CommandStatus, Environment,
    Session, SessionSummary,
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
    #[cfg(feature = "clipboard")]
    #[arg(short, long)]
    copy: bool,
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

fn collect_commands(sessions: &[SessionSummary]) -> Vec<String> {
    sessions.iter().flat_map(|session| session.records.iter().map(|r| r.command.clone())).collect()
}

fn lookup_commands<I: IntoIterator<Item = S>, S: AsRef<str>>(
    references: I,
    session_names: &[String],
) -> Result<Vec<String>> {
    let resolved =
        resolve_references(references, session_names).context("could not resolve references")?;
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

pub fn show_to(references: &[String], script: bool, mut out: impl Write) -> Result<()> {
    let mut iter = references.iter();

    while let Some(reference) = iter.next() {
        let session = read_session(reference).context("could not read session data")?;
        if script {
            print_session_script(session, &mut out, stderr()).context("could not print output")?;
        } else {
            print_session(session, &mut out, stderr()).context("could not print output")?;
        }
        if iter.len() > 0 {
            writeln!(&mut out)?;
        }
    }

    Ok(())
}

pub fn show(action: ShowAction) -> Result<()> {
    let ShowAction { script, session: reference_args, .. } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let references: Vec<String> = match reference_args.is_empty() && !session_names.is_empty() {
        true => vec![session_names[0].clone()],
        false => resolve_references(reference_args.iter(), &session_names)
            .context("invalid `--session` argument")?,
    };

    #[cfg(feature = "clipboard")]
    if action.copy {
        let mut cursor = std::io::Cursor::new(Vec::new());
        show_to(&references, script, &mut cursor)?;
        let buffer = cursor.into_inner();
        let text = String::from_utf8_lossy(&buffer);
        let len = text.len();
        arboard::Clipboard::new()
            .and_then(|mut cb| cb.set_text(text))
            .context("could not set text to clipboard")?;
        eprintln!("{} chars copied into clipboard", len);
        return Ok(());
    }

    show_to(&references, script, stdout())
}

pub fn list(action: ListAction) -> Result<()> {
    let ListAction { full, limit } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let limit = limit.min(session_names.len());

    for (index, reference) in session_names[0..limit].iter().enumerate() {
        let session = read_session(reference).context("could not read session data")?;
        let key = index + 1;
        let max = (!full).then_some(5);
        print_session_brief(session, key, max, stdout()).context("could not print output")?;
        println!();
    }

    println!("({} / {} sessions)", limit, session_names.len());

    Ok(())
}

pub fn remove(action: RemoveAction) -> Result<()> {
    let RemoveAction { all, session: reference_args } = action;

    let session_names = list_session_names().context("could not list sessions")?;
    let references: Vec<String> = match all {
        true => session_names,
        false => resolve_references(reference_args.iter(), &session_names)
            .context("invalid `--session` argument")?,
    };

    for reference in &references {
        remove_session(reference).context("could not remove session")?;
        println!("session {} removed", reference);
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
