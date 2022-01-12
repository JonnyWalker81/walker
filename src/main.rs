use std::{
    io::{self, SeekFrom},
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

use tui_input::backend::crossterm as input_backend;
use tui_input::{Input, InputResponse, StateChanged};
use unicode_width::UnicodeWidthStr;

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
                        _ => {}
                    }
                            }
                            InputMode::Editing(ref _kind) => {
                                match event.code {
                                KeyCode::Esc => app.set_input_mode(InputMode::Normal),
                                _ => {
                                    let resp = input_backend::to_input_request(CEvent::Key(event))
                                    .and_then(|req| app.text_input_mut().handle(req));

                    match resp {
                        Some(InputResponse::StateChanged(_)) => {}
                        Some(InputResponse::Submitted) => {
                            match _kind {
                                EditingKind::Rename => {
                                app.rename_file();
                                }
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

#[derive(Copy, Clone, Debug)]
enum EditingKind {
    Rename,
}

#[derive(Copy, Clone, Debug)]
enum InputMode {
    Normal,
    Editing(EditingKind),
}

#[derive(Debug)]
struct State {
    current_dir: String,
    directory_table_state: TableState,
    current_contents: Vec<Item>,
    is_editing: bool,
    file_to_edit: Item,
    editing_index: usize,
    input_mode: InputMode,
    text_input: Input,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current_dir: String::new(),
            directory_table_state: TableState::default(),
            current_contents: vec![],
            is_editing: false,
            file_to_edit: Item::default(),
            editing_index: 0,
            input_mode: InputMode::Normal,
            text_input: Input::default(),
        }
    }
}

#[derive(Clone, Debug)]
struct Item {
    name: String,
    size: usize,
    perms: String,
    modified_date: DateTime<Local>,
    is_dir: bool,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            name: String::new(),
            size: 0,
            perms: String::new(),
            modified_date: Local.ymd(1970, 1, 1).and_hms(0, 0, 0),
            is_dir: false,
        }
    }
}

impl Item {
    fn new() -> Self {
        Self::default()
    }

    fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    fn with_size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    fn with_perms(mut self, perms: &str) -> Self {
        self.perms = perms.to_string();
        self
    }

    fn is_dir(mut self, dir: bool) -> Self {
        self.is_dir = dir;
        self
    }
}

#[derive(Debug)]
struct App {
    state: State,
}

impl Default for App {
    fn default() -> Self {
        Self {
            state: State::default(),
        }
    }
}

impl App {
    fn new() -> Self {
        Self {
            state: State::default(),
        }
    }

    fn set_current_dir(&mut self, dir: &str) {
        let path = Path::new(dir);
        if path.is_dir() {
            self.state.current_dir = dir.to_string();
            self.load_dir();
        }
    }

    fn current_dir(&self) -> &String {
        &self.state.current_dir
    }

    fn current_contents(&self) -> &[Item] {
        &self.state.current_contents
    }

    fn set_directory_table_state(&mut self, state: TableState) {
        self.state.directory_table_state = state;
    }

    fn directory_table_state(&self) -> &TableState {
        &self.state.directory_table_state
    }

    fn directory_table_state_mut(&mut self) -> &mut TableState {
        &mut self.state.directory_table_state
    }

    fn text_input(&self) -> &Input {
        &self.state.text_input
    }

    fn text_input_mut(&mut self) -> &mut Input {
        &mut self.state.text_input
    }

    fn is_editing(&self) -> bool {
        self.state.is_editing
    }

    fn input_mode(&self) -> InputMode {
        self.state.input_mode
    }

    fn load_dir(&mut self) -> Result<()> {
        self.state.current_contents = get_contents(&self.state.current_dir)?;
        Ok(())
    }

    fn move_selection_up(&mut self) {
        if let Some(selected) = self.state.directory_table_state.selected() {
            if selected > 0 {
                self.state.directory_table_state.select(Some(selected - 1));
            } else {
                self.state
                    .directory_table_state
                    .select(Some(self.state.current_contents.len() - 1));
            }
        }
    }

    fn move_selection_down(&mut self) {
        if let Some(selected) = self.state.directory_table_state.selected() {
            if selected >= self.state.current_contents.len() - 1 {
                self.state.directory_table_state.select(Some(0));
            } else {
                self.state.directory_table_state.select(Some(selected + 1));
            }
        }
    }

    fn move_into_child_dir(&mut self) {
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(item) = self.state.current_contents.get(idx) {
                let full_path = Path::new(&self.state.current_dir).join(&item.name);
                self.set_current_dir(&full_path.display().to_string());
                self.state.directory_table_state.select(Some(0));
            }
        }
    }

    fn move_upto_parent_dir(&mut self) {
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(parent) = Path::new(&self.state.current_dir.clone()).parent() {
                self.set_current_dir(&parent.display().to_string());
                self.state.directory_table_state.select(Some(0));
            }
        }
    }

    fn start_rename_file(&mut self) {
        self.state.is_editing = true;
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(selected_item) = self.state.current_contents.get(idx) {
                let path = Path::new(&selected_item.name);
                self.state.file_to_edit = selected_item.clone();
                self.state.input_mode = InputMode::Editing(EditingKind::Rename);
                self.state.text_input = self
                    .state
                    .text_input
                    .clone()
                    .with_value(self.state.file_to_edit.name.clone());
            }
        }
    }

    fn set_input_mode(&mut self, input_mode: InputMode) {
        match input_mode {
            InputMode::Normal => {
                self.state.file_to_edit = Item::default();
                self.state.input_mode = input_mode;
                self.state.is_editing = false;
            }
            InputMode::Editing(_) => {}
        }
    }

    fn rename_file(&mut self) {
        let name: String = self.state.text_input.value().into();
        std::fs::rename(&self.state.file_to_edit.name, &name);
        self.set_input_mode(InputMode::Normal);
        self.state.directory_table_state.select(Some(0));
        self.load_dir();
    }
}

fn get_contents(path: &str) -> Result<Vec<Item>> {
    // for f in WalkDir::new(path).max_depth(0) {
    //     contents.push(f?.path().display().to_string());
    // }

    // FIXME: Remove use of unwrap
    let contents = WalkDir::new(path)
        .sort_by_file_name()
        .max_depth(1)
        .into_iter()
        .map(|f| Item::new().with_name(&f.unwrap().path().display().to_string()))
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
    let titles = vec![app.current_dir().as_str()]
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
        .current_contents()
        .iter()
        .map(|f| Row::new(vec![Cell::from(Span::raw(f.name.to_string()))]))
        .collect();

    let file_table = Table::new(rows)
        .widths(&[Constraint::Percentage(100)])
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(file_table, chunks[1], app.directory_table_state_mut());

    let width = chunks[0].width.max(3) - 3; // keep 2 for borders and 1 for cursor
    let scroll = (app.text_input().cursor() as u16).max(width) - width;
    if app.is_editing() {
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
        InputMode::Normal =>
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            {}

        InputMode::Editing(_) => {
            // Make the cursor visible and ask tui-rs to put it at the specified coordinates after rendering
            f.set_cursor(
                // Put cursor past the end of the input text
                chunks[2].x + (app.text_input().cursor() as u16).min(width) + 1,
                // Move one line down, from the border to the input line
                chunks[2].y + 1,
            )
        }
    }
    // match app.tabs.index {
    //     0 => draw_first_tab(f, app, chunks[1]),
    //     1 => draw_second_tab(f, app, chunks[1]),
    //     2 => draw_third_tab(f, app, chunks[1]),
    //     _ => {}
    // };
    Ok(())
}
