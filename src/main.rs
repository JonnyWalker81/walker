use humansize::{file_size_opts as options, FileSize};
use std::{
    default,
    io::{self, SeekFrom},
    os::unix::prelude::{CommandExt, PermissionsExt},
    panic,
    path::Path,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, TimeZone, Utc};
use tokio::sync::mpsc::{Receiver, Sender};

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};

use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs, Wrap},
    Frame, Terminal,
};
use walkdir::WalkDir;

use crate::app::{App, EditingKind, InputMode};
use tui_input::backend::crossterm as input_backend;
use tui_input::{Input, InputResponse, StateChanged};
use unicode_width::UnicodeWidthStr;

mod app;
mod view;

#[derive(Parser, Debug)]
#[clap(version = "1.0", author = "Jonathan Rothberg")]
struct Args {
    #[clap(subcommand)]
    subcmd: Option<SubCommand>,
}

#[derive(Subcommand, Debug)]
enum SubCommand {}

#[tokio::main]
async fn main() -> Result<()> {
    let _args = Args::parse();

    match run(_args).await {
        Ok(_) => {}
        Err(e) => println!("{:?}", e),
    }

    Ok(())
}

async fn run(_args: Args) -> Result<()> {
    match _args {
        _ => run_ui().await?,
    }
    Ok(())
}

async fn run_ui() -> Result<()> {
    enable_raw_mode()?;

    panic::set_hook(Box::new(|info| {
        println!("Panic: {}", info);
        disable_raw_mode().expect("restore terminal raw mode");
    }));

    let mut rx = start_key_events();
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    io::stdout().execute(EnterAlternateScreen)?;
    terminal.clear()?;

    let (mut ui_tx, mut ui_rx): (Sender<Event<KeyEvent>>, Receiver<Event<KeyEvent>>) =
        tokio::sync::mpsc::channel(1);

    let mut app = App::new();
    let current_dir = std::env::current_dir()?;
    app.set_current_dir(&current_dir.display().to_string());
    let mut table_state = TableState::default();
    table_state.select(Some(0));
    app.set_directory_table_state(table_state);

    loop {
        terminal.draw(|rect| {
            let _ = draw(rect, &mut app);
        });

        tokio::select! {
            Some(event) = rx.recv() =>{
                match event {
                    Event::Input(event) =>
                        match app.input_mode() {
                            InputMode::Normal => {
                                match event.code {
                                    KeyCode::Char('q') => {
                                        disable_raw_mode()?;
                                        io::stdout().execute(LeaveAlternateScreen)?;
                                        terminal.show_cursor()?;
                                            break;
                                    },
                                    KeyCode::Down | KeyCode::Char('j') => app.move_selection_down(),
                                    KeyCode::Up | KeyCode::Char('k') => app.move_selection_up(),
                                    KeyCode::Right | KeyCode::Char('l') => app.move_into_child_dir(),
                                    KeyCode::Left | KeyCode::Char('h') => app.move_upto_parent_dir(),
                                    KeyCode::Char('r') => app.start_rename_file(),
                                    KeyCode::Char('y') => app.initiate_file_copy(),
                                    _ => {}
                                }
                            }
                            InputMode::Editing(ref _kind) => {
                                match event.code {
                                    KeyCode::Esc => app.set_input_mode(InputMode::Normal),
                                    KeyCode::Down | KeyCode::Char('j') => app.move_selection_down(),
                                    KeyCode::Up | KeyCode::Char('k') => app.move_selection_up(),
                                    KeyCode::Right | KeyCode::Char('l') => app.move_into_child_dir(),
                                    KeyCode::Left | KeyCode::Char('h') => app.move_upto_parent_dir(),
                                    _ => {
                                        let resp = input_backend::to_input_request(CEvent::Key(event))
                                        .and_then(|req| app.text_input_mut().handle(req));

                                        match resp {
                                            Some(InputResponse::StateChanged(_)) => {}
                                            Some(InputResponse::Submitted) => {
                                                if let EditingKind::Rename = _kind {
                                                    app.rename_file();
                                                }
                                            }

                                            Some(InputResponse::Escaped) => {
                                                app.set_input_mode(InputMode::Normal);
                                            }
                                            None => {}
                                        }
                                    }
                                }
                            }
                        }
                    Event::Tick => {}
                }
            }
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
pub enum Event<I> {
    Input(I),
    Tick,
}

fn start_key_events() -> tokio::sync::mpsc::Receiver<Event<KeyEvent>> {
    let (mut tx, mut rx) = tokio::sync::mpsc::channel(1);
    let tick_rate = Duration::from_millis(200);
    tokio::spawn(async move {
        let mut last_tick = Instant::now();
        loop {
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if event::poll(timeout).expect("poll works") {
                if let CEvent::Key(key) = event::read().expect("can read events") {
                    let _ = tx.send(Event::Input(key)).await;
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if let Ok(_) = tx.send(Event::Tick).await {
                    last_tick = Instant::now();
                }
            }
        }
    });

    rx
}

fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) -> Result<()> {
    let chunks = Layout::default()
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.size());
    let titles = vec![app.main_panel_mut().current_dir().as_str()]
        .iter()
        .map(|t| {
            Spans::from(Span::styled(
                t.to_string(),
                Style::default().fg(Color::Green),
            ))
        })
        .collect();
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Walker"))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(0);
    f.render_widget(tabs, chunks[0]);

    let rows: Vec<_> = app
        .main_panel()
        .current_contents()
        .iter()
        .map(|f| -> Row {
            Row::new(vec![
                Cell::from(Span::raw(f.name.to_string())),
                Cell::from(Span::raw(f.perms.to_string())),
                Cell::from(Span::raw(
                    f.size.file_size(options::DECIMAL).unwrap_or_default(),
                )),
            ])
        })
        .collect();

    let body_chunks = if app.input_mode().is_copy() {
        Layout::default()
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .direction(Direction::Horizontal)
            // .margin(1)
            .split(chunks[1])
    } else {
        Layout::default()
            .constraints([Constraint::Percentage(100)].as_ref())
            // .margin(1)
            .split(chunks[1])
    };

    let file_table = Table::new(rows)
        .widths(&[
            Constraint::Percentage(75),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
        ])
        .column_spacing(10)
        .highlight_style(
            Style::default()
                .fg(Color::Rgb(0, 0, 0))
                .bg(Color::Rgb(0, 125, 255))
                .add_modifier(Modifier::BOLD),
        );
    // f.render_stateful_widget(file_table, body_chunks[0], app.directory_table_state_mut());
    f.render_stateful_widget(
        file_table,
        body_chunks[0],
        app.main_panel_mut().directory_table_state_mut(),
    );

    if app.input_mode().is_copy() {
        // let selected_dir = app
        //     .main_panel()
        //     .selected_item()
        //     .map_or(String::new(), |i| i.name.clone());
        // app.action_panel_mut().set_current_dir(&selected_dir);
        let action_rows: Vec<_> = app
            .action_panel()
            .current_contents()
            .iter()
            .map(|f| -> Row {
                Row::new(vec![
                    Cell::from(Span::raw(f.name.to_string())),
                    Cell::from(Span::raw(f.perms.to_string())),
                    Cell::from(Span::raw(
                        f.size.file_size(options::DECIMAL).unwrap_or_default(),
                    )),
                ])
            })
            .collect();

        let action_table = Table::new(action_rows)
            .widths(&[
                Constraint::Percentage(75),
                Constraint::Percentage(12),
                Constraint::Percentage(12),
            ])
            .column_spacing(10)
            .highlight_style(
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Rgb(0, 125, 255))
                    .add_modifier(Modifier::BOLD),
            );

        f.render_stateful_widget(
            action_table,
            body_chunks[1],
            app.action_panel_mut().directory_table_state_mut(),
        );
    }
    // f.render_stateful_widget(file_table, body_chunks[1], app.directory_table_state_mut());

    let width = chunks[0].width.max(3) - 3; // keep 2 for borders and 1 for cursor
    let scroll = (app.text_input().cursor() as u16).max(width) - width;
    if app.input_mode().is_renaming() {
        // let text = vec![Spans::from(app.file_to_edit.clone())];
        let input = Paragraph::new(app.text_input().value())
            .style(match app.input_mode() {
                InputMode::Normal => Style::default(),
                InputMode::Editing(_) => Style::default().fg(Color::Yellow),
            })
            .scroll((0, scroll))
            .block(Block::default().borders(Borders::ALL).title("Rename"));
        // let block = Block::default().borders(Borders::ALL).title(Span::styled(
        //     "Rename",
        //     Style::default()
        //         .fg(Color::Magenta)
        //         .add_modifier(Modifier::BOLD),
        // ));
        // f.render_widget(paragraph, chunks[2]);
        f.render_widget(input, chunks[2]);
    } else {
        let text = vec![Spans::from("")];
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            "Normal",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[2]);
    }

    match app.input_mode() {
        InputMode::Editing(EditingKind::Rename) => {
            // Make the cursor visible and ask tui-rs to put it at the specified coordinates after rendering
            f.set_cursor(
                // Put cursor past the end of the input text
                chunks[2].x + (app.text_input().cursor() as u16).min(width) + 1,
                // Move one line down, from the border to the input line
                chunks[2].y + 1,
            )
        }
        // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
        _ => {}
    }
    // match app.tabs.index {
    //     0 => draw_first_tab(f, app, chunks[1]),
    //     1 => draw_second_tab(f, app, chunks[1]),
    //     2 => draw_third_tab(f, app, chunks[1]),
    //     _ => {}
    // };
    Ok(())
}
