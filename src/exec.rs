use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::iter::Iterator;

use anyhow::{bail, Context, Result};
use duct::cmd;
use tempfile::TempDir;

#[derive(Debug, Default, PartialEq)]
pub struct Environment {
    env_vars: Option<Vec<(String, String)>>,
    work_dir: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct CommandResult {
    pub new_env: Environment,
    pub output: String,
    pub succeeded: bool,
}

pub fn parse_env_file<B: BufRead>(content: &mut B) -> Result<Environment> {
    let mut env_vars = Vec::new();

    loop {
        let mut buf = Vec::new();
        let n = content.read_until(b'\0', &mut buf).context("could not read env file")?;
        if n == 0 {
            break;
        }
        if buf.last().copied() == Some(b'\0') {
            buf.pop();
        }
        let text = String::from_utf8(buf).context("could not parse env file as utf8")?;
        let mut parts = text.splitn(2, '=');
        let (name, value) = match (parts.next(), parts.next()) {
            (Some(name), Some(value)) => (name, value),
            _ => bail!("unexpected env file format"),
        };
        env_vars.push((name.to_owned(), value.to_owned()));
    }

    let work_dir = env_vars.iter().find(|(k, _)| k == "PWD").map(|(_, v)| v.clone());

    Ok(Environment { env_vars: Some(env_vars), work_dir })
}

pub fn execute(cmd: &str, env: Environment, mut out: impl Write) -> Result<CommandResult> {
    let temp_dir = TempDir::new().context("could not create temporary directory")?;
    let env_path = temp_dir.path().join("env");

    let cmd = format!(r#"trap "env -0 > $(printf %q "$1")" EXIT; {}"#, cmd);
    let mut prog = cmd!("bash", "-c", cmd, "bash", env_path.as_os_str())
        .stdin_null()
        .stderr_to_stdout()
        .unchecked();

    if let Some(work_dir) = env.work_dir {
        prog = prog.dir(work_dir);
    }
    if let Some(env_vars) = env.env_vars {
        prog = prog.full_env(env_vars);
    }

    let mut reader = prog.reader().context("could not execute `bash`")?;

    let mut output = Vec::new();
    let mut buffer = [0; 8192];

    loop {
        let n = reader.read(&mut buffer).context("could not read command output")?;
        if n == 0 {
            break;
        }
        let read = &buffer[0..n];
        output.extend(read);
        out.write_all(read)?;
    }

    let status = match reader.try_wait()? {
        Some(o) => o.status,
        None => bail!("unexpected EOF while reading command output"),
    };

    let env_file = File::open(env_path).context("could not open env file")?;
    let new_env =
        parse_env_file(&mut BufReader::new(env_file)).context("could not parse `env` output")?;

    Ok(CommandResult {
        new_env,
        output: String::from_utf8_lossy(&output).to_string(),
        succeeded: status.success(),
    })
}

#[cfg(test)]
mod test {

    use super::*;
    use std::io::{BufReader, Cursor};
    use std::path::Path;

    #[test]
    fn test_parse_env_file() {
        let content = b"abc=123\0abc=456\0xyz=123\n456\n789\n";
        let actual = parse_env_file(&mut BufReader::new(Cursor::new(content)));
        let expected = Environment {
            env_vars: Some(vec![
                ("abc".into(), "123".into()),
                ("abc".into(), "456".into()),
                ("xyz".into(), "123\n456\n789\n".into()),
            ]),
            work_dir: None,
        };
        assert!(actual.is_ok());
        assert_eq!(expected, actual.unwrap());
    }

    #[test]
    fn test_parse_env_file_pwd() {
        let content = b"PWD=/path/to/pwd";
        let actual = parse_env_file(&mut BufReader::new(Cursor::new(content)));
        let expected = Environment {
            env_vars: Some(vec![("PWD".into(), "/path/to/pwd".into())]),
            work_dir: Some("/path/to/pwd".into()),
        };
        assert!(actual.is_ok());
        assert_eq!(expected, actual.unwrap());
    }

    fn assert_eq_result(expected: &CommandResult, actual: &CommandResult) {
        let CommandResult { new_env: Environment { env_vars, work_dir }, output, succeeded } =
            actual;
        assert_eq!(&expected.new_env.work_dir, work_dir);
        assert_eq!(&expected.output, output);
        assert_eq!(&expected.succeeded, succeeded);

        let expected_env_vars = expected.new_env.env_vars.as_ref();

        assert_eq!(expected_env_vars.is_some(), env_vars.is_some());
        if let Some(expected_env_vars) = expected_env_vars {
            let env_vars = env_vars.as_ref().unwrap();
            for v in expected_env_vars.iter() {
                assert!(env_vars.contains(v));
                assert!(env_vars.iter().filter(|w| v.0 == w.0).all(|w| v.1 == w.1));
            }
        }
    }

    #[test]
    fn test_execute() {
        let path_to_string = |p: &Path| p.to_str().unwrap().to_owned();

        let temp_dir = tempfile::tempdir().unwrap();
        let temp_path = temp_dir.path();
        let sub_path = temp_path.join("sub");

        let cmd = "mkdir sub && cd sub && export ABC=123 && echo $ABC && echo 456 > abc";
        let env = Environment {
            env_vars: Some(vec![("PWD".to_owned(), path_to_string(temp_path))]),
            work_dir: Some(path_to_string(temp_path)),
        };
        let mut out = Vec::new();

        let actual = execute(cmd, env, &mut out);
        let expected = CommandResult {
            new_env: Environment {
                env_vars: Some(vec![
                    ("PWD".to_owned(), path_to_string(&sub_path)),
                    ("ABC".to_owned(), "123".to_owned()),
                ]),
                work_dir: Some(path_to_string(&sub_path)),
            },
            output: "123\n".into(),
            succeeded: true,
        };

        assert_eq!(Some(expected.output.clone()), String::from_utf8(out).ok());
        assert!(actual.is_ok());
        assert_eq_result(&expected, &actual.unwrap());
    }

    #[test]
    fn test_execute_failed_command() {
        let path_to_string = |p: &Path| p.to_str().unwrap().to_owned();

        let temp_dir = tempfile::tempdir().unwrap();
        let temp_path = temp_dir.path();

        let cmd = "echo 123 && false";
        let env = Environment {
            env_vars: Some(vec![("PWD".to_owned(), path_to_string(temp_path))]),
            work_dir: Some(path_to_string(temp_path)),
        };
        let mut out = Vec::new();

        let actual = execute(cmd, env, &mut out);
        let expected = CommandResult {
            new_env: Environment {
                env_vars: Some(vec![("PWD".to_owned(), path_to_string(temp_path))]),
                work_dir: Some(path_to_string(temp_path)),
            },
            output: "123\n".into(),
            succeeded: false,
        };

        assert_eq!(Some(expected.output.clone()), String::from_utf8(out).ok());
        assert!(actual.is_ok());
        assert_eq_result(&expected, &actual.unwrap());
    }
}
