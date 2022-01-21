use std::{os::unix::prelude::PermissionsExt, path::Path};

use crate::view::WalkerView;
use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};
use tui::widgets::TableState;
use tui_input::Input;
use walkdir::WalkDir;

#[derive(Copy, Clone, Debug)]
pub enum EditingKind {
    Rename,
    Copy,
}

#[derive(Copy, Clone, Debug)]
pub enum InputMode {
    Normal,
    Editing(EditingKind),
}

#[derive(Copy, Clone, Debug)]
pub enum PanelKind {
    Main,
    Secondary,
}

impl InputMode {
    pub fn is_copy(&self) -> bool {
        matches!(*self, InputMode::Editing(EditingKind::Copy))
    }

    pub fn is_renaming(&self) -> bool {
        matches!(*self, InputMode::Editing(EditingKind::Rename))
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) perms: String,
    pub(crate) modified_date: DateTime<Local>,
    pub(crate) is_dir: bool,
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

    fn with_size(mut self, size: u64) -> Self {
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
struct State {
    active_panel: PanelKind,
    main_view: WalkerView,
    action_view: WalkerView,
}

impl Default for State {
    fn default() -> Self {
        Self {
            active_panel: PanelKind::Main,
            main_view: WalkerView::default(),
            action_view: WalkerView::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct App {
    state: State,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: State::default(),
        }
    }

    pub fn set_current_dir(&mut self, dir: &str) {
        self.get_active_view_mut().set_current_dir(dir);
    }

    pub fn current_dir(&self) -> &String {
        self.get_active_view().current_dir()
    }

    pub fn current_contents(&self) -> &[Item] {
        self.get_active_view().current_contents()
    }

    pub fn set_directory_table_state(&mut self, state: TableState) {
        self.get_active_view_mut().set_directory_table_state(state);
    }

    pub fn directory_table_state(&self) -> &TableState {
        self.get_active_view().directory_table_state()
    }

    pub fn directory_table_state_mut(&mut self) -> &mut TableState {
        self.get_active_view_mut().directory_table_state_mut()
    }

    pub fn text_input(&self) -> &Input {
        self.get_active_view().text_input()
    }

    pub fn text_input_mut(&mut self) -> &mut Input {
        self.get_active_view_mut().text_input_mut()
    }

    pub fn is_editing(&self) -> bool {
        self.get_active_view().is_editing()
    }

    pub fn input_mode(&self) -> InputMode {
        self.get_active_view().input_mode()
    }

    pub fn load_dir(&mut self) -> Result<()> {
        self.get_active_view_mut().load_dir();
        Ok(())
    }

    fn get_active_view(&self) -> &WalkerView {
        match self.state.active_panel {
            PanelKind::Main => &self.state.main_view,
            PanelKind::Secondary => &self.state.active_panel,
        }
    }

    fn get_active_view_mut(&mut self) -> &mut WalkerView {
        match self.state.active_panel {
            PanelKind::Main => &mut self.state.main_view,
            PanelKind::Secondary => &mut self.state.active_panel,
        }
    }

    pub fn move_selection_up(&mut self) {
        self.get_active_view_mut().move_selection_up();
    }

    pub fn move_selection_down(&mut self) {
        self.get_active_view_mut().move_selection_down();
    }

    pub fn move_into_child_dir(&mut self) {
        self.get_active_view_mut().move_into_child_dir();
    }

    pub fn move_upto_parent_dir(&mut self) {
        self.get_active_view_mut().move_upto_parent_dir();
    }

    pub fn start_rename_file(&mut self) {
        self.get_active_view_mut().start_rename_file();
    }

    pub fn set_input_mode(&mut self, input_mode: InputMode) {
        self.get_active_view_mut().set_input_mode(input_mode);
    }

    pub fn rename_file(&mut self) {
        self.get_active_view_mut().rename_file();
    }

    pub fn initiate_file_copy(&mut self) {
        self.get_active_view_mut().initiate_file_copy();
    }
}

pub fn get_contents(path: &str) -> Result<Vec<Item>> {
    // FIXME: Remove use of unwrap
    let contents = WalkDir::new(path)
        .sort_by_file_name()
        .max_depth(1)
        .into_iter()
        .map(|ref f| {
            let perms = format!(
                "{:o}",
                f.as_ref().unwrap().metadata().unwrap().permissions().mode()
            );
            let perms_octal: u32 = u32::from_str_radix(&perms, 8).unwrap();

            Item::new()
                .with_name(&(f.as_ref().unwrap().path().display().to_string()))
                .with_size(f.as_ref().unwrap().metadata().unwrap().len())
                .with_perms(&unix_mode::to_string(perms_octal))
        })
        .skip(1)
        .collect();
    Ok(contents)
}
