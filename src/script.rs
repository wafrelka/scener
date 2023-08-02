use std::fs::File;
use std::io::{stdin, BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};

pub fn read_script<B: BufRead>(reader: B) -> Result<Vec<String>> {
    let is_empty = |line: &String| {
        let line = line.trim();
        line.is_empty() || line.starts_with("#!")
    };

    reader
        .lines()
        .filter(|line| !line.as_ref().is_ok_and(is_empty))
        .map(|line| line.context("could not read line"))
        .collect()
}

pub fn read_script_from_stdin() -> Result<Vec<String>> {
    read_script(BufReader::new(stdin())).context("could not read script from STDIN")
}

pub fn read_script_from_files<I: Iterator<Item = P>, P: AsRef<Path>>(
    paths: I,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for path in paths.into_iter() {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("could not open script file at {}", path.display()))?;
        let script = read_script(BufReader::new(file))
            .with_context(|| format!("could not read script from {}", path.display()))?;
        lines.extend(script);
    }
    Ok(lines)
}

#[cfg(test)]
mod test {

    use tempfile::TempDir;

    use super::*;
    use std::fs::write;
    use std::io::{BufReader, Cursor};

    #[test]
    fn test_read_script() {
        let content = b"abc\ndef\n";
        let actual = read_script(BufReader::new(Cursor::new(content)));
        let expected = Some(vec!["abc".to_owned(), "def".to_owned()]);
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_read_script_filter_empty_lines() {
        let content = b"   abc   \n   \n   #! shebang   \n   def   \n";
        let actual = read_script(BufReader::new(Cursor::new(content)));
        let expected = Some(vec!["   abc   ".to_owned(), "   def   ".to_owned()]);
        assert_eq!(expected, actual.ok());
    }

    #[test]
    fn test_read_script_from_files() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        write(temp_path.join("file1"), b"abc\ndef\n").unwrap();
        write(temp_path.join("file2"), b"ghi\njkl\n").unwrap();

        let actual =
            read_script_from_files([temp_path.join("file1"), temp_path.join("file2")].iter());
        let expected: Option<Vec<String>> =
            Some(vec!["abc", "def", "ghi", "jkl"].into_iter().map(ToOwned::to_owned).collect());
        assert_eq!(expected, actual.ok());
    }
}
