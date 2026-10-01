//! Workspace selection and the trust confirmation shown before file tools
//! get access to the working directory.
use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

/// Expand a leading `~` (or `~/`) to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Ask the user to trust `workspace` before handing file tools access to it.
/// Returns `true` when they continue, `false` when they decline (exit).
///
/// In a real terminal this is an arrow-key menu (↑/↓ + Enter); when stdin or
/// stdout is redirected it falls back to a plain yes/no line prompt so scripts
/// and tests keep working.
pub fn prompt_workspace_trust(editor: &mut DefaultEditor, workspace: &Path) -> Result<bool> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        prompt_workspace_trust_menu(workspace)
    } else {
        prompt_workspace_trust_line(editor, workspace)
    }
}

fn prompt_workspace_trust_line(editor: &mut DefaultEditor, workspace: &Path) -> Result<bool> {
    println!("Accessing workspace: {}", workspace.display());
    println!(
        "Quick safety check: Is this a project you created or own code, a well-known open source project, or work for a company you trust?"
    );
    loop {
        let answer = match editor.readline("Trust this folder? (yes/no) > ") {
            Ok(line) => line.trim().to_ascii_lowercase(),
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        match answer.as_str() {
            "yes" | "y" | "trust" => return Ok(true),
            "no" | "n" | "exit" => return Ok(false),
            _ => println!("Please answer yes or no."),
        }
    }
}

fn prompt_workspace_trust_menu(workspace: &Path) -> Result<bool> {
    use crossterm::{
        cursor::MoveUp,
        event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
        execute,
        style::{Print, PrintStyledContent, Stylize},
        terminal,
    };

    let mut stdout = io::stdout();
    // Leave a blank line after the providers banner.
    execute!(stdout, Print("\r\n"))?;

    terminal::enable_raw_mode()?;

    // Vertical, flush-left option list with a `>` marker on the selected row
    // (matching the reference trust dialog). The safer choice ("No, exit")
    // is listed first and selected by default.
    const OPTIONS: [&str; 2] = ["No, exit", "Yes, I trust this folder"];
    let mut selected = 0usize;

    // NOTE: raw mode disables the terminal's OPOST processing, so `\n` no
    // longer returns to column 0. Every line break here must be an explicit
    // `\r\n` or each line would start where the previous one ended (a
    // staircase effect). A stray `\r` is harmless when OPOST is still on.
    //
    // The menu is drawn once; keypresses then rewrite only the fixed-size
    // options block in place via MoveUp(4) (2 options + blank + hint). We
    // deliberately do NOT use SavePosition/RestorePosition + clear: once the
    // menu scrolls the terminal, the saved DEC position is no longer adjusted
    // by some terminals, so each redraw would land mid-menu and stack stale
    // copies below the old one.
    let draw_options = |stdout: &mut io::Stdout, selected: usize| -> Result<()> {
        for (i, label) in OPTIONS.iter().enumerate() {
            if i == selected {
                execute!(
                    stdout,
                    PrintStyledContent("> ".magenta()),
                    PrintStyledContent(label.bold()),
                    Print("\r\n"),
                )?;
            } else {
                execute!(stdout, Print(format!("  {}\r\n", label)))?;
            }
        }
        execute!(stdout, Print("\r\nEnter to confirm · Esc to cancel\r\n"))?;
        Ok(())
    };

    // First (full) draw: description, then the options block.
    execute!(
        stdout,
        PrintStyledContent("Accessing workspace:".yellow().bold()),
        Print("\r\n\r\n"),
        PrintStyledContent(workspace.display().to_string().bold()),
        Print("\r\n\r\n"),
        Print("Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open source project, or work from your team). If not, take a moment to review what's in this folder first.\r\n\r\n"),
        Print("rsmgo will be able to read, edit, and execute files here.\r\n\r\n"),
        PrintStyledContent("Security guide".underlined()),
        Print("\r\n\r\n"),
    )?;
    draw_options(&mut stdout, selected)?;

    let result = (|| -> Result<bool> {
        loop {
            match event::read()? {
                Event::Key(KeyEvent { code: KeyCode::Left | KeyCode::Up, .. }) => {
                    selected = (selected + OPTIONS.len() - 1) % OPTIONS.len();
                }
                Event::Key(KeyEvent { code: KeyCode::Right | KeyCode::Down, .. }) => {
                    selected = (selected + 1) % OPTIONS.len();
                }
                Event::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
                    return Ok(OPTIONS[selected] == "Yes, I trust this folder");
                }
                Event::Key(KeyEvent { code: KeyCode::Esc, .. })
                | Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }) => return Ok(false),
                // Any other key leaves the selection unchanged; no redraw.
                _ => continue,
            }
            // Rewrite the 4-line options block in place: the cursor sits one
            // row below the hint line, so move up 4 rows to the first option.
            execute!(stdout, MoveUp(4))?;
            draw_options(&mut stdout, selected)?;
        }
    })();

    terminal::disable_raw_mode()?;
    result
}
