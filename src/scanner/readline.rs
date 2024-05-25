use anyhow::{Context, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::cell::{OnceCell, RefCell};

thread_local! {
    static EDITOR: RefCell<OnceCell<DefaultEditor>> = RefCell::new(OnceCell::new());
}

fn scan_line_with_editor(editor: &mut DefaultEditor) -> Result<Option<String>> {
    let history_path = crate::get_history_path()?;

    loop {
        match editor.readline("==> ") {
            Ok(line) => {
                editor.add_history_entry(&line).context("could not update line editor history")?;
                let _ = editor.append_history(&history_path); // TODO: print warning message
                return Ok(Some(line));
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                return Ok(None);
            }
            Err(ReadlineError::WindowResized) => {
                continue;
            }
            Err(err) => return Err(err).context("could not read command from STDIN"),
        }
    }
}

pub fn scan_line() -> Result<Option<String>> {
    EDITOR.with_borrow_mut(|cell| -> Result<Option<String>> {
        if cell.get().is_none() {
            let his = crate::get_history_path()?;
            let mut editor = DefaultEditor::new().context("could not initialize line editor")?;
            let _ = editor.load_history(&his); // TODO: print warning message
            cell.get_or_init(|| editor);
        }
        let editor = cell.get_mut().unwrap();
        scan_line_with_editor(editor)
    })
}
