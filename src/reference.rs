#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReferenceError {
    #[error("index out of range (ref = {reference})")]
    IndexOutOfRange { reference: String },
    #[error("session not found (ref = {reference})")]
    SessionNotFound { reference: String },
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

pub fn resolve_reference(
    reference: impl AsRef<str>,
    session_names: &[String],
) -> Result<String, ReferenceError> {
    let reference = reference.as_ref();
    if let Some(index) = parse_index(reference) {
        let name = session_names
            .get(index)
            .ok_or(ReferenceError::IndexOutOfRange { reference: reference.to_owned() })?;
        Ok(name.clone())
    } else {
        let found = session_names.iter().any(|name| name == reference);
        if found {
            Ok(reference.to_owned())
        } else {
            Err(ReferenceError::SessionNotFound { reference: reference.to_owned() })
        }
    }
}

pub fn resolve_references<I: IntoIterator<Item = S>, S: AsRef<str>>(
    references: I,
    session_names: &[String],
) -> Result<Vec<String>, ReferenceError> {
    references.into_iter().map(|r| resolve_reference(r.as_ref(), session_names)).collect()
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use super::ReferenceError::*;
    use super::*;

    #[rstest]
    #[case::bare("@", Some(0))]
    #[case::zero("@0", None)]
    #[case::one("@1", Some(0))]
    #[case::five("@5", Some(4))]
    #[case::invalid("@abc", None)]
    fn test_parse_index(#[case] s: &str, #[case] expected: Option<usize>) {
        assert_eq!(parse_index(s), expected);
    }

    #[rstest]
    #[case::by_index("@2", Ok("test2".into()))]
    #[case::index_out_of_range("@3", Err(IndexOutOfRange{ reference: "@3".into() }))]
    #[case::by_name("test1", Ok("test1".into()))]
    #[case::name_not_found("test3", Err(SessionNotFound { reference: "test3".into() }))]
    fn test_resolve_reference(#[case] r: &str, #[case] expected: Result<String, ReferenceError>) {
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_reference(r, &names);
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::ok(
        vec!["@1".into(), "@2".into()],
        Ok(vec!["test1".into(), "test2".into()]),
    )]
    #[case::ok(
        vec!["@1".into(), "@3".into(), "invalid".into()],
        Err(IndexOutOfRange{ reference: "@3".into() }),
    )]
    fn test_resolve_references(
        #[case] r: Vec<String>,
        #[case] expected: Result<Vec<String>, ReferenceError>,
    ) {
        let names = vec!["test1".into(), "test2".into()];
        let actual = resolve_references(r, &names);
        assert_eq!(actual, expected);
    }
}
