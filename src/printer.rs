use std::io::Write;

use chrono::{DateTime, Local, Utc};

use crate::{CommandStatus, Session};

pub fn needs_newline(s: &str) -> bool {
    !s.is_empty() && !s.ends_with('\n')
}

fn format_datetime(dt: DateTime<Utc>) -> String {
    let local: DateTime<Local> = dt.into();
    local.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn print_session(
    session: Session,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> std::io::Result<()> {
    writeln!(&mut stderr, "session {} ({})", session.name, format_datetime(session.recorded_at))?;

    let iter = session.records.into_iter();
    let iter = iter.filter(|r| r.status.is_executed());
    let mut iter = iter.peekable();

    while let Some(record) = iter.next() {
        writeln!(&mut stdout, "$ {}", record.command)?;
        write!(&mut stdout, "{}", record.output)?;
        if needs_newline(&record.output) {
            writeln!(&mut stdout)?;
        }
        if iter.peek().is_some() {
            writeln!(&mut stdout)?;
        }
    }

    Ok(())
}

pub fn print_session_script(
    session: Session,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> std::io::Result<()> {
    writeln!(&mut stderr, "session {} ({})", session.name, format_datetime(session.recorded_at))?;
    for record in session.records.into_iter() {
        writeln!(&mut stdout, "{}", record.command)?;
    }
    Ok(())
}

pub fn print_session_brief(
    session: Session,
    key: usize,
    max: Option<usize>,
    mut stdout: impl Write,
) -> std::io::Result<()> {
    writeln!(&mut stdout, "{}: {} ({})", key, session.name, format_datetime(session.recorded_at))?;

    let len = session.records.len();
    let n = max.unwrap_or(len).min(len);
    let rem = len - n;

    for record in session.records.iter().take(n) {
        let marker = match record.status {
            CommandStatus::Succeeded | CommandStatus::Failed => "$",
            CommandStatus::Skipped => "?",
        };
        writeln!(&mut stdout, "    {} {}", marker, record.command)?;
    }
    if rem > 0 {
        writeln!(&mut stdout, "    ... ({} more commands)", rem)?;
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use chrono::TimeZone;
    use indoc::indoc;
    use rstest::rstest;

    use crate::CommandRecord;

    use super::*;

    #[rstest]
    #[case::empty("", false)]
    #[case::with_newline("abc\ndef\n", false)]
    #[case::without_newline("abc\ndef", true)]
    fn test_needs_newline_empty(#[case] s: String, #[case] expected: bool) {
        assert_eq!(needs_newline(&s), expected);
    }

    fn good_session() -> Session {
        Session {
            name: "session-name".into(),
            recorded_at: Local.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap().into(),
            records: vec![
                CommandRecord {
                    command: "echo hello".into(),
                    output: "hello\n".into(),
                    status: CommandStatus::Succeeded,
                },
                CommandRecord {
                    command: "echo -n world".into(),
                    output: "world".into(),
                    status: CommandStatus::Succeeded,
                },
                CommandRecord {
                    command: "echo \"hello, world!\"".into(),
                    output: "hello, world!\n".into(),
                    status: CommandStatus::Succeeded,
                },
            ],
        }
    }

    fn bad_session() -> Session {
        Session {
            name: "session-name".into(),
            recorded_at: Local.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap().into(),
            records: vec![
                CommandRecord {
                    command: "echo hello".into(),
                    output: "hello\n".into(),
                    status: CommandStatus::Succeeded,
                },
                CommandRecord {
                    command: "echo -n world".into(),
                    output: "world".into(),
                    status: CommandStatus::Failed,
                },
                CommandRecord {
                    command: "echo \"hello, world!\"".into(),
                    output: "hello, world!\n".into(),
                    status: CommandStatus::Skipped,
                },
            ],
        }
    }

    #[rstest]
    #[case::good(
        good_session(),
        indoc! {r#"
            $ echo hello
            hello

            $ echo -n world
            world

            $ echo "hello, world!"
            hello, world!
        "#},
        "session session-name (2020-01-02 03:04:05)\n",
    )]
    #[case::bad(
        bad_session(),
        indoc! {r#"
            $ echo hello
            hello

            $ echo -n world
            world
        "#},
        "session session-name (2020-01-02 03:04:05)\n",
    )]
    fn test_print_session(
        #[case] session: Session,
        #[case] expected_out: &str,
        #[case] expected_err: &str,
    ) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        print_session(session, &mut out, &mut err).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), expected_out);
        assert_eq!(String::from_utf8(err).unwrap(), expected_err);
    }

    #[rstest]
    #[case::good(
        good_session(),
        indoc! {r#"
            echo hello
            echo -n world
            echo "hello, world!"
        "#},
        "session session-name (2020-01-02 03:04:05)\n",
    )]
    #[case::bad(
        bad_session(),
        indoc! {r#"
            echo hello
            echo -n world
            echo "hello, world!"
        "#},
        "session session-name (2020-01-02 03:04:05)\n",
    )]
    fn test_print_session_script(
        #[case] session: Session,
        #[case] expected_out: &str,
        #[case] expected_err: &str,
    ) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        print_session_script(session, &mut out, &mut err).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), expected_out);
        assert_eq!(String::from_utf8(err).unwrap(), expected_err);
    }

    #[rstest]
    #[case::good(
        good_session(),
        None,
        indoc! {r#"
            123: session-name (2020-01-02 03:04:05)
                $ echo hello
                $ echo -n world
                $ echo "hello, world!"
        "#}.trim_start(),
    )]
    #[case::bad(
        bad_session(),
        None,
        indoc! {r#"
            123: session-name (2020-01-02 03:04:05)
                $ echo hello
                $ echo -n world
                ? echo "hello, world!"
        "#},
    )]
    #[case::max(
        good_session(),
        Some(1),
        indoc! {r#"
            123: session-name (2020-01-02 03:04:05)
                $ echo hello
                ... (2 more commands)
        "#}.trim_start(),
    )]
    fn test_print_session_brief(
        #[case] session: Session,
        #[case] max: Option<usize>,
        #[case] expected: &str,
    ) {
        let mut out = Vec::new();
        print_session_brief(session, 123, max, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }
}
