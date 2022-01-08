use std::{
    io::{self, SeekFrom},
    panic,
    path::Path,
    time::{Duration, Instant},
};

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
                    Event::Input(event) => match event.code {
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
                        _ => {}
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

struct App {
    current_dir: String,
    directory_table_state: TableState,
    current_contents: Vec<String>,
}

impl App {
    fn new() -> Self {
        Self {
            current_dir: String::new(),
            directory_table_state: TableState::default(),
            current_contents: vec![],
        }
    }

    fn set_current_dir(&mut self, dir: &str) {
        self.current_dir = dir.to_string();
        self.load_dir();
    }

    fn set_directory_table_state(&mut self, state: TableState) {
        self.directory_table_state = state;
    }

    fn load_dir(&mut self) -> Result<()> {
        self.current_contents = get_contents(&self.current_dir)?;
        Ok(())
    }

    fn move_selection_up(&mut self) {
        if let Some(selected) = self.directory_table_state.selected() {
            if selected > 0 {
                self.directory_table_state.select(Some(selected - 1));
            } else {
                self.directory_table_state
                    .select(Some(self.current_contents.len() - 1));
            }
        }
    }

    fn move_selection_down(&mut self) {
        if let Some(selected) = self.directory_table_state.selected() {
            if selected >= self.current_contents.len() - 1 {
                self.directory_table_state.select(Some(0));
            } else {
                self.directory_table_state.select(Some(selected + 1));
            }
        }
    }

    fn move_into_child_dir(&mut self) {
        if let Some(idx) = self.directory_table_state.selected() {
            if let Some(name) = self.current_contents.get(idx) {
                let full_path = Path::new(&self.current_dir).join(name);
                self.set_current_dir(&full_path.display().to_string());
            }
        }
    }

    fn move_upto_parent_dir(&mut self) {
        if let Some(idx) = self.directory_table_state.selected() {
            if let Some(parent) = Path::new(&self.current_dir.clone()).parent() {
                self.set_current_dir(&parent.display().to_string());
            }
        }
    }
}

fn get_contents(path: &str) -> Result<Vec<String>> {
    // for f in WalkDir::new(path).max_depth(0) {
    //     contents.push(f?.path().display().to_string());
    // }

    // FIXME: Remove use of unwrap
    let contents = WalkDir::new(path)
        .max_depth(1)
        .into_iter()
        .map(|f| f.unwrap().path().display().to_string())
        .skip(1)
        .collect();
    Ok(contents)
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
    let titles = vec![app.current_dir.as_str()]
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
        .current_contents
        .iter()
        .map(|f| Row::new(vec![Cell::from(Span::raw(f.to_string()))]))
        .collect();

    let file_table = Table::new(rows)
        .widths(&[Constraint::Percentage(100)])
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(file_table, chunks[1], &mut app.directory_table_state);

    let text = vec![Spans::from(
        "This is a paragraph with several lines. You can change style your text the way you want",
    )];
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Footer",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    ));
    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
    f.render_widget(paragraph, chunks[2]);
    // match app.tabs.index {
    //     0 => draw_first_tab(f, app, chunks[1]),
    //     1 => draw_second_tab(f, app, chunks[1]),
    //     2 => draw_third_tab(f, app, chunks[1]),
    //     _ => {}
    // };
    Ok(())
}
