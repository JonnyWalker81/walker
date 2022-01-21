use anyhow::Result;
use std::path::Path;
use tui::widgets::TableState;
use tui_input::Input;

use crate::app::{get_contents, EditingKind, InputMode, Item};

#[derive(Clone, Debug)]
pub struct WalkerState {
    current_dir: String,
    directory_table_state: TableState,
    current_contents: Vec<Item>,
    is_editing: bool,
    file_to_edit: Item,
    editing_index: usize,
    input_mode: InputMode,
    text_input: Input,
}

impl Default for WalkerState {
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
pub struct WalkerView {
    state: WalkerState,
}

impl Default for WalkerView {
    fn default() -> Self {
        Self {
            state: WalkerState::default(),
        }
    }
}

impl WalkerView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_current_dir(&mut self, dir: &str) {
        let path = Path::new(dir);
        if path.is_dir() {
            self.state.current_dir = dir.to_string();
            self.load_dir();
        }
    }

    pub fn current_dir(&self) -> &String {
        &self.state.current_dir
    }

    pub fn current_contents(&self) -> &[Item] {
        &self.state.current_contents
    }

    pub fn load_dir(&mut self) -> Result<()> {
        self.state.current_contents = get_contents(&self.state.current_dir)?;
        Ok(())
    }

    pub fn state(&self) -> &WalkerState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut WalkerState {
        &mut self.state
    }

    pub fn directory_table_state(&self) -> &TableState {
        &self.state.directory_table_state
    }

    pub fn directory_table_state_mut(&mut self) -> &mut TableState {
        &mut self.state.directory_table_state
    }

    pub fn set_directory_table_state(&mut self, state: TableState) {
        self.state.directory_table_state = state;
    }

    pub fn text_input(&self) -> &Input {
        &self.state.text_input
    }

    pub fn text_input_mut(&mut self) -> &mut Input {
        &mut self.state.text_input
    }

    pub fn is_editing(&self) -> bool {
        self.state.is_editing
    }

    pub fn input_mode(&self) -> InputMode {
        self.state.input_mode
    }

    pub fn move_selection_up(&mut self) {
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

    pub fn move_selection_down(&mut self) {
        if let Some(selected) = self.state.directory_table_state.selected() {
            if selected >= self.state.current_contents.len() - 1 {
                self.state.directory_table_state.select(Some(0));
            } else {
                self.state.directory_table_state.select(Some(selected + 1));
            }
        }
    }

    pub fn move_into_child_dir(&mut self) {
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(item) = self.state.current_contents.get(idx) {
                let full_path = Path::new(&self.state.current_dir).join(&item.name);
                self.set_current_dir(&full_path.display().to_string());
                self.state.directory_table_state.select(Some(0));
            }
        }
    }

    pub fn move_upto_parent_dir(&mut self) {
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(parent) = Path::new(&self.state.current_dir.clone()).parent() {
                self.set_current_dir(&parent.display().to_string());
                self.state.directory_table_state.select(Some(0));
            }
        }
    }

    pub fn start_rename_file(&mut self) {
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

    pub fn set_input_mode(&mut self, input_mode: InputMode) {
        match input_mode {
            InputMode::Normal => {
                self.state.file_to_edit = Item::default();
                self.state.input_mode = input_mode;
                self.state.is_editing = false;
            }
            InputMode::Editing(_) => {}
        }
    }

    pub fn rename_file(&mut self) {
        let name: String = self.state.text_input.value().into();
        std::fs::rename(&self.state.file_to_edit.name, &name);
        self.set_input_mode(InputMode::Normal);
        self.state.directory_table_state.select(Some(0));
        self.load_dir();
    }

    pub fn initiate_file_copy(&mut self) {
        self.state.is_editing = true;
        if let Some(idx) = self.state.directory_table_state.selected() {
            if let Some(selected_item) = self.state.current_contents.get(idx) {
                let path = Path::new(&selected_item.name);
                self.state.file_to_edit = selected_item.clone();
                self.state.input_mode = InputMode::Editing(EditingKind::Copy);
            }
        }
    }
}
