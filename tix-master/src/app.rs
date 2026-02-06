use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget, Clear},
    Frame,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct SlaveInfo {
    pub ip: String,
    pub ram_usage: String,
    pub other: Vec<String>,
}

#[derive(Debug, Default)]
pub struct MasterInfo {
    pub ip: String,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
}

#[derive(Debug, Clone)]
pub enum MasterEvent {
    Log(String),
    SlaveConnected(String),
    SlaveInfo { ram_usage: String },
    TaskUpdate { id: u64, status: String },
    TreeData { is_slave: bool, path: String, data: String },
    RefreshTree { is_slave: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompletionType {
    Command,
    Path,
}

#[derive(Debug, Clone)]
pub struct CompletionOption {
    pub display: String,
    pub value: String,
    pub is_dir: bool,
}

#[derive(Debug, Default)]
pub struct CompletionState {
    pub options: Vec<CompletionOption>,
    pub selected_index: usize,
    pub active: bool,
    pub trigger_type: Option<CompletionType>,
    pub last_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Main,
    TreeExplorer,
    SystemSettings,
}

#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub children: Option<Vec<FileNode>>,
    pub is_selected: bool,
}

#[derive(Debug, Default)]
pub struct TreeViewState {
    pub root_nodes: Vec<FileNode>,
    pub cursor_index: usize,
    pub scroll_offset: usize,
}

#[derive(Debug, Default)]
pub struct TreeExplorerState {
    pub local_tree: TreeViewState,
    pub slave_tree: TreeViewState,
    pub active_side: bool, // false = local, true = slave
    pub clipboard: Vec<PathBuf>,
    pub is_cut_operation: bool,
}

#[derive(Debug)]
pub struct App {
    pub master_info: MasterInfo,
    pub slave_info: SlaveInfo,
    pub tasks: Vec<String>,
    pub command_to_execute: String,
    pub logs: Vec<String>,
    pub log_scroll: usize,
    pub autoscroll: bool,
    pub completion: CompletionState,
    pub exit: bool,
    pub available_commands: Vec<String>,
    pub last_input_time: std::time::Instant,
    pub needs_completion_update: bool,
    pub active_tab: Tab,
    pub tree_explorer: TreeExplorerState,
}

impl App {
    pub fn new() -> Self {
        Self {
            master_info: MasterInfo {
                ip: "10.0.0.1".to_string(),
            },
            slave_info: SlaveInfo {
                ip: "Not Connected".to_string(),
                ram_usage: "N/A".to_string(),
                other: Vec::new(),
            },
            tasks: Vec::new(),
            command_to_execute: String::new(),
            logs: vec![
                "Welcome to Tix Master".to_string(),
                "Waiting for connections...".to_string(),
            ],
            log_scroll: 0,
            autoscroll: true,
            completion: CompletionState::default(),
            exit: false,
            available_commands: vec![
                "Ping".to_string(),
                "HelloWorld".to_string(),
                "ShellExecute".to_string(),
                "Copy".to_string(),
                "Exit".to_string(),
            ],
            last_input_time: std::time::Instant::now(),
            needs_completion_update: false,
            active_tab: Tab::Main,
            tree_explorer: TreeExplorerState::default(),
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
        if tab == Tab::TreeExplorer {
            if self.tree_explorer.local_tree.root_nodes.is_empty() {
                self.refresh_local_drives();
            }
            if self.tree_explorer.slave_tree.root_nodes.is_empty() {
                self.refresh_slave_drives();
            }
        }
    }

    fn refresh_local_drives(&mut self) {
        let mut drives = Vec::new();
        // In a real Windows environment, we'd list A-Z, but for now let's start with C:
        for drive in ["C:\\", "D:\\", "E:\\"] {
            let path = PathBuf::from(drive);
            if path.exists() {
                drives.push(FileNode {
                    name: drive.to_string(),
                    path,
                    is_dir: true,
                    is_expanded: false,
                    children: None,
                    is_selected: false,
                });
            }
        }
        self.tree_explorer.local_tree.root_nodes = drives;
    }

    pub fn tree_refresh(&mut self) -> Option<String> {
        let active_side = self.tree_explorer.active_side;
        let tree = if !active_side { &mut self.tree_explorer.local_tree } else { &mut self.tree_explorer.slave_tree };
        
        // If the tree is empty, refresh drives
        if tree.root_nodes.is_empty() {
            if !active_side {
                self.refresh_local_drives();
                return None;
            } else {
                return Some("ListDrives".to_string());
            }
        }

        // Find current path at cursor
        let mut current_idx = 0;
        let mut current_path = None;
        Self::get_path_at_cursor_static(&tree.root_nodes, tree.cursor_index, &mut current_idx, &mut current_path);

        if let Some(path) = current_path {
            if !active_side {
                // Local refresh
                if let Some(node) = Self::find_node_mut(&mut tree.root_nodes, &path) {
                    if node.is_dir && node.is_expanded {
                        Self::load_node_children_static(node);
                        self.logs.push(format!("Refreshed local directory: {}", path.display()));
                    } else if let Some(parent_path) = path.parent() {
                        if let Some(parent_node) = Self::find_node_mut(&mut tree.root_nodes, parent_path) {
                            Self::load_node_children_static(parent_node);
                            self.logs.push(format!("Refreshed local parent directory: {}", parent_path.display()));
                        }
                    }
                }
            } else {
                // Slave refresh
                let refresh_path = if let Some(node) = Self::find_node_at_path_static(&tree.root_nodes, &path) {
                    if node.is_dir && node.is_expanded {
                        path
                    } else {
                        path.parent().unwrap_or(Path::new("")).to_path_buf()
                    }
                } else {
                    path
                };

                if !refresh_path.as_os_str().is_empty() {
                    let path_str = refresh_path.to_string_lossy().to_string();
                    self.logs.push(format!("Refreshing slave directory: {}", path_str));
                    return Some(format!("ListDir {}", path_str));
                } else {
                    return Some("ListDrives".to_string());
                }
            }
        } else {
            // Fallback to drives
            if !active_side {
                self.refresh_local_drives();
            } else {
                return Some("ListDrives".to_string());
            }
        }
        None
    }

    fn find_node_at_path_static<'a>(nodes: &'a [FileNode], path: &Path) -> Option<&'a FileNode> {
        for node in nodes {
            if node.path == path {
                return Some(node);
            }
            if let Some(children) = &node.children {
                if let Some(found) = Self::find_node_at_path_static(children, path) {
                    return Some(found);
                }
            }
        }
        None
    }

    pub fn refresh_slave_drives(&mut self) -> Option<String> {
        self.logs.push("Requesting drives from slave...".to_string());
        Some("ListDrives".to_string())
    }

    pub fn tree_cursor_down(&mut self) {
        let active_side = self.tree_explorer.active_side;
        let (root_nodes, cursor_index, _) = if !active_side {
            (&self.tree_explorer.local_tree.root_nodes, &mut self.tree_explorer.local_tree.cursor_index, &mut self.tree_explorer.local_tree.scroll_offset)
        } else {
            (&self.tree_explorer.slave_tree.root_nodes, &mut self.tree_explorer.slave_tree.cursor_index, &mut self.tree_explorer.slave_tree.scroll_offset)
        };
        
        let mut count = 0;
        Self::count_visible_static(root_nodes, &mut count);
        if *cursor_index + 1 < count {
            *cursor_index += 1;
        }
    }

    pub fn tree_cursor_up(&mut self) {
        let active_side = self.tree_explorer.active_side;
        let cursor_index = if !active_side {
            &mut self.tree_explorer.local_tree.cursor_index
        } else {
            &mut self.tree_explorer.slave_tree.cursor_index
        };

        if *cursor_index > 0 {
            *cursor_index -= 1;
        }
    }

    pub fn tree_toggle_expand(&mut self) -> Option<String> {
        let active_side = self.tree_explorer.active_side;
        let (root_nodes, cursor_index) = if !active_side {
            (&mut self.tree_explorer.local_tree.root_nodes, self.tree_explorer.local_tree.cursor_index)
        } else {
            (&mut self.tree_explorer.slave_tree.root_nodes, self.tree_explorer.slave_tree.cursor_index)
        };

        let mut current_idx = 0;
        let mut node_to_load = None;
        
        Self::toggle_node_at_static(root_nodes, cursor_index, &mut current_idx, &mut node_to_load, active_side);
        
        if let Some(path) = node_to_load {
            if !active_side {
                // Find node again to load children (to satisfy borrow checker)
                if let Some(node) = Self::find_node_mut(root_nodes, &path) {
                    Self::load_node_children_static(node);
                }
            } else {
                let path_str = path.to_string_lossy().to_string();
                self.logs.push(format!("Requesting directory listing for slave: {}", path_str));
                return Some(format!("ListDir {}", path_str));
            }
        }
        None
    }

    fn load_node_children_static(node: &mut FileNode) {
        if let Ok(entries) = std::fs::read_dir(&node.path) {
            let mut children = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = path.is_dir();
                children.push(FileNode {
                    name,
                    path,
                    is_dir,
                    is_expanded: false,
                    children: None,
                    is_selected: false,
                });
            }
            children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
            node.children = Some(children);
        }
    }

    fn toggle_node_at_static(nodes: &mut Vec<FileNode>, target_idx: usize, current_idx: &mut usize, node_to_load: &mut Option<PathBuf>, is_slave: bool) -> bool {
        for node in nodes {
            if *current_idx == target_idx {
                if node.is_dir {
                    node.is_expanded = !node.is_expanded;
                    if node.is_expanded && node.children.is_none() {
                        *node_to_load = Some(node.path.clone());
                    }
                }
                return true;
            }
            *current_idx += 1;
            if node.is_expanded {
                if let Some(children) = &mut node.children {
                    if Self::toggle_node_at_static(children, target_idx, current_idx, node_to_load, is_slave) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn find_node_mut<'a>(nodes: &'a mut Vec<FileNode>, path: &Path) -> Option<&'a mut FileNode> {
        for node in nodes {
            if node.path == path {
                return Some(node);
            }
            if let Some(children) = &mut node.children {
                if let Some(found) = Self::find_node_mut(children, path) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn count_visible_static(nodes: &[FileNode], count: &mut usize) {
        for node in nodes {
            *count += 1;
            if node.is_expanded {
                if let Some(children) = &node.children {
                    Self::count_visible_static(children, count);
                }
            }
        }
    }

    pub fn tree_toggle_select(&mut self) {
        let active_side = self.tree_explorer.active_side;
        let (root_nodes, cursor_index) = if !active_side {
            (&mut self.tree_explorer.local_tree.root_nodes, self.tree_explorer.local_tree.cursor_index)
        } else {
            (&mut self.tree_explorer.slave_tree.root_nodes, self.tree_explorer.slave_tree.cursor_index)
        };

        let mut current_idx = 0;
        Self::select_node_at_static(root_nodes, cursor_index, &mut current_idx);
    }

    fn select_node_at_static(nodes: &mut Vec<FileNode>, target_idx: usize, current_idx: &mut usize) -> bool {
        for node in nodes {
            if *current_idx == target_idx {
                node.is_selected = !node.is_selected;
                return true;
            }
            *current_idx += 1;
            if node.is_expanded {
                if let Some(children) = &mut node.children {
                    if Self::select_node_at_static(children, target_idx, current_idx) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn tree_copy(&mut self) {
        let active_side = self.tree_explorer.active_side;
        let root_nodes = if !active_side {
            &self.tree_explorer.local_tree.root_nodes
        } else {
            &self.tree_explorer.slave_tree.root_nodes
        };

        let mut selected = Vec::new();
        self.get_selected_paths(root_nodes, &mut selected);
        
        if !selected.is_empty() {
            self.tree_explorer.clipboard = selected;
            self.tree_explorer.is_cut_operation = false;
            self.logs.push(format!("Copied {} items to clipboard", self.tree_explorer.clipboard.len()));
        }
    }

    pub fn tree_cut(&mut self) {
        let active_side = self.tree_explorer.active_side;
        let root_nodes = if !active_side {
            &self.tree_explorer.local_tree.root_nodes
        } else {
            &self.tree_explorer.slave_tree.root_nodes
        };

        let mut selected = Vec::new();
        self.get_selected_paths(root_nodes, &mut selected);

        if !selected.is_empty() {
            self.tree_explorer.clipboard = selected;
            self.tree_explorer.is_cut_operation = true;
            self.logs.push(format!("Cut {} items to clipboard", self.tree_explorer.clipboard.len()));
        }
    }

    fn get_selected_paths(&self, nodes: &[FileNode], out: &mut Vec<PathBuf>) {
        for node in nodes {
            if node.is_selected {
                out.push(node.path.clone());
            }
            if let Some(children) = &node.children {
                self.get_selected_paths(children, out);
            }
        }
    }

    pub fn tree_switch_side(&mut self) {
        self.tree_explorer.active_side = !self.tree_explorer.active_side;
    }

    pub fn tree_paste(&mut self) -> Vec<String> {
        let mut commands = Vec::new();
        if self.tree_explorer.clipboard.is_empty() {
            self.logs.push("Clipboard is empty".to_string());
            return commands;
        }

        let active_side = self.tree_explorer.active_side;
        let dest_tree = if !active_side { &self.tree_explorer.local_tree } else { &self.tree_explorer.slave_tree };
        
        // Find the current directory at cursor or use root
        let mut current_idx = 0;
        let mut dest_path = None;
        Self::get_path_at_cursor_static(&dest_tree.root_nodes, dest_tree.cursor_index, &mut current_idx, &mut dest_path);
        
        let dest_dir = if let Some(path) = dest_path {
            if path.is_dir() { path } else { path.parent().unwrap_or(Path::new("")).to_path_buf() }
        } else if !dest_tree.root_nodes.is_empty() {
            dest_tree.root_nodes[0].path.clone()
        } else {
            self.logs.push("Error: Could not determine destination directory".to_string());
            return commands;
        };

        let dest_dir_str = dest_dir.to_string_lossy().to_string();
        let is_upload = !self.tree_explorer.active_side; // False if pasting INTO local (download), True if pasting INTO slave (upload)
        // Wait, active_side: false = local, true = slave.
        // If active_side is true, we are on slave side, so we want to paste INTO slave (Upload).
        // If active_side is false, we are on local side, so we want to paste INTO local (Download).
        let is_paste_to_slave = active_side;
        
        // Determine if source is also on the same side
        // For simplicity, we assume if we are on local side, we only paste local paths if they are local
        // and if we are on slave side, we only paste slave paths if they are slave.
        // But the clipboard doesn't currently store which side the paths came from.
        // Let's assume for now:
        // - If dest is local and all paths are absolute windows paths, it's a local copy.
        // - If dest is slave, we always use Upload for now (since we don't know if src was slave).
        
        let mut local_copy_count = 0;

        for src_path in &self.tree_explorer.clipboard {
            let src_path_str = src_path.to_string_lossy().to_string();
            
            if is_paste_to_slave {
                // Upload: Local -> Slave
                self.logs.push(format!("Uploading {} to {}", src_path_str, dest_dir_str));
                commands.push(format!("Upload {}|{}", src_path_str, dest_dir_str));
            } else {
                // Dest is Local.
                // If it's a local-to-local copy:
                if src_path.exists() {
                    let mut dest_file = dest_dir.clone();
                    if let Some(file_name) = src_path.file_name() {
                        dest_file.push(file_name);
                        self.logs.push(format!("Copying local {} to {}", src_path_str, dest_file.display()));
                        if src_path.is_dir() {
                            // Simplified directory copy
                            let _ = self.copy_dir_all(src_path, &dest_file);
                        } else {
                            let _ = std::fs::copy(src_path, &dest_file);
                        }
                        local_copy_count += 1;
                    }
                } else {
                    // Download: Slave -> Local
                    self.logs.push(format!("Downloading {} to {}", src_path_str, dest_dir_str));
                    commands.push(format!("Download {}|{}", src_path_str, dest_dir_str));
                }
            }
        }

        if local_copy_count > 0 {
            self.tree_refresh();
        }

        if self.tree_explorer.is_cut_operation {
            // In a real app, we'd delete after successful copy. For now just clear.
            self.tree_explorer.clipboard.clear();
        }
        
        commands
    }

    fn get_path_at_cursor_static(nodes: &[FileNode], target_idx: usize, current_idx: &mut usize, found_path: &mut Option<PathBuf>) -> bool {
        for node in nodes {
            if *current_idx == target_idx {
                *found_path = Some(node.path.clone());
                return true;
            }
            *current_idx += 1;
            if node.is_expanded {
                if let Some(children) = &node.children {
                    if Self::get_path_at_cursor_static(children, target_idx, current_idx, found_path) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn copy_dir_all(&self, src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                self.copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
            } else {
                std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    pub fn on_input_change(&mut self) {
        self.last_input_time = std::time::Instant::now();
        self.needs_completion_update = true;
    }

    pub fn update_completion(&mut self) {
        if !self.needs_completion_update {
            return;
        }
        self.needs_completion_update = false;
        self.trigger_completion();
    }

    pub fn handle_tab(&mut self) {
        if self.completion.active && !self.completion.options.is_empty() {
            self.completion.selected_index = (self.completion.selected_index + 1) % self.completion.options.len();
        } else {
            self.trigger_completion();
        }
    }

    pub fn handle_up(&mut self) {
        if self.completion.active && !self.completion.options.is_empty() {
            if self.completion.selected_index == 0 {
                self.completion.selected_index = self.completion.options.len() - 1;
            } else {
                self.completion.selected_index -= 1;
            }
        } else {
            self.log_scroll = (self.log_scroll + 1).min(self.logs.len().saturating_sub(1));
            self.autoscroll = false;
        }
    }

    pub fn handle_down(&mut self) {
        if self.completion.active && !self.completion.options.is_empty() {
            self.completion.selected_index = (self.completion.selected_index + 1) % self.completion.options.len();
        } else {
            if self.log_scroll > 0 {
                self.log_scroll -= 1;
            }
            if self.log_scroll == 0 {
                self.autoscroll = true;
            }
        }
    }

    pub fn handle_enter(&mut self) -> Option<String> {
        if self.completion.active && !self.completion.options.is_empty() {
            self.apply_completion();
            self.completion.active = false;
            None
        } else if !self.command_to_execute.is_empty() {
            let cmd = self.command_to_execute.clone();
            self.command_to_execute.clear();
            self.completion.active = false;
            Some(cmd)
        } else {
            None
        }
    }

    pub fn handle_esc(&mut self) {
        if self.completion.active {
            self.completion.active = false;
        } else {
            self.exit = true;
        }
    }

    fn trigger_completion(&mut self) {
        let input = &self.command_to_execute;
        
        // Command autocomplete (first word)
        if !input.contains(' ') {
            self.completion.trigger_type = Some(CompletionType::Command);
            let mut options = Vec::new();
            for cmd in &self.available_commands {
                if cmd.to_lowercase().starts_with(&input.to_lowercase()) {
                    options.push(CompletionOption {
                        display: cmd.clone(),
                        value: cmd.clone(),
                        is_dir: false,
                    });
                }
            }
            if !options.is_empty() {
                self.completion.options = options;
                self.completion.selected_index = 0;
                self.completion.active = true;
                return;
            }
        }

        // Path autocomplete
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() > 1 || (parts.len() == 1 && input.ends_with(' ')) {
            self.completion.trigger_type = Some(CompletionType::Path);
            let last_word = if input.ends_with(' ') { "" } else { parts.last().unwrap_or(&"") };
            
            // Special handling for directory trigger: path ending with \ preceded by char
            let is_dir_trigger = input.ends_with('\\') && input.len() > 1 && !input.chars().rev().nth(1).unwrap().is_whitespace();

            let path_to_scan = if is_dir_trigger {
                last_word
            } else if last_word.contains('\\') || last_word.contains('/') {
                last_word
            } else {
                "./"
            };

            let mut entries = Vec::new();
            let (dir, prefix) = if is_dir_trigger {
                (PathBuf::from(path_to_scan), "")
            } else if let Some(parent) = Path::new(path_to_scan).parent() {
                let prefix = Path::new(path_to_scan).file_name().and_then(|f| f.to_str()).unwrap_or("");
                (parent.to_path_buf(), prefix)
            } else {
                (PathBuf::from("./"), last_word)
            };

            if let Ok(read_dir) = std::fs::read_dir(&dir) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                        let is_dir = entry.path().is_dir();
                        entries.push(CompletionOption {
                            display: name.clone(),
                            value: name,
                            is_dir,
                        });
                    }
                }
            }

            if !entries.is_empty() {
                entries.sort_by(|a, b| a.display.cmp(&b.display));
                self.completion.options = entries;
                self.completion.selected_index = 0;
                self.completion.active = true;
            } else {
                self.completion.active = false;
            }
        } else {
            self.completion.active = false;
        }
    }

    fn apply_completion(&mut self) {
        if let Some(choice) = self.completion.options.get(self.completion.selected_index) {
            let input = &self.command_to_execute;
            
            if self.completion.trigger_type == Some(CompletionType::Command) {
                self.command_to_execute = choice.value.clone();
            } else {
                let parts: Vec<&str> = input.split_whitespace().collect();
                let mut new_cmd = if input.ends_with(' ') {
                    input.to_string()
                } else {
                    let p = parts[..parts.len()-1].join(" ");
                    if p.is_empty() { String::new() } else { format!("{} ", p) }
                };

                // If it was a path completion, we need to handle the directory prefix
                if !input.ends_with(' ') {
                    let last_word = parts.last().unwrap_or(&"");
                    if let Some(parent) = Path::new(last_word).parent() {
                        let parent_str = parent.to_string_lossy().to_string();
                        if !parent_str.is_empty() && parent_str != "." {
                            new_cmd.push_str(&parent_str);
                            if !parent_str.ends_with('\\') && !parent_str.ends_with('/') {
                                new_cmd.push('\\');
                            }
                        }
                    }
                }
                
                new_cmd.push_str(&choice.value);
                if choice.is_dir {
                    new_cmd.push('\\');
                }
                self.command_to_execute = new_cmd;
            }
        }
    }

    pub fn update(&mut self, event: MasterEvent) {
        match event {
            MasterEvent::Log(msg) => {
                // Split multi-line messages into individual lines
                for line in msg.lines() {
                    self.logs.push(line.to_string());
                }
                if self.autoscroll {
                    self.log_scroll = 0; // Reset scroll to show latest (bottom)
                }
            }
            MasterEvent::SlaveConnected(ip) => {
                self.slave_info.ip = ip;
                self.logs.push(format!("Slave connected: {}", self.slave_info.ip));
            }
            MasterEvent::SlaveInfo { ram_usage } => {
                self.slave_info.ram_usage = ram_usage;
            }
            MasterEvent::TaskUpdate { id, status } => {
                let id_str = format!("{}", id);
                if let Some(task) = self.tasks.iter_mut().find(|t| t.contains(&format!("< {} >", id_str))) {
                    *task = format!("< {} > {}", id_str, status);
                } else {
                    self.tasks.push(format!("< {} > {}", id_str, status));
                }
            }
            MasterEvent::TreeData { is_slave, path, data } => {
                if is_slave {
                    if path == "drives" {
                        let drives: Vec<FileNode> = data.split(',')
                            .filter(|s| !s.is_empty())
                            .map(|s| FileNode {
                                name: s.to_string(),
                                path: PathBuf::from(s),
                                is_dir: true,
                                is_expanded: false,
                                children: None,
                                is_selected: false,
                            })
                            .collect();
                        self.tree_explorer.slave_tree.root_nodes = drives;
                    } else if path == "dir_listing" {
                        // Parse data: "PATH|/some/path;name1|0|123;name2|1|0"
                        let mut entries: Vec<&str> = data.split(';').collect();
                        if entries.is_empty() { return; }

                        let mut target_path = PathBuf::new();
                        let mut startIndex = 0;

                        if entries[0].starts_with("PATH|") {
                            target_path = PathBuf::from(&entries[0][5..]);
                            startIndex = 1;
                        }

                        let children: Vec<FileNode> = entries[startIndex..].iter()
                            .filter(|s| !s.is_empty())
                            .filter_map(|s| {
                                let parts: Vec<&str> = s.split('|').collect();
                                if parts.len() >= 2 {
                                    let name = parts[0].to_string();
                                    let is_dir = parts[1] == "1";
                                    let mut full_path = target_path.clone();
                                    full_path.push(&name);
                                    Some(FileNode {
                                        name,
                                        path: full_path,
                                        is_dir,
                                        is_expanded: false,
                                        children: None,
                                        is_selected: false,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        
                        if !target_path.as_os_str().is_empty() {
                            // Update specific node
                            if let Some(node) = Self::find_node_mut(&mut self.tree_explorer.slave_tree.root_nodes, &target_path) {
                                let mut updated_children = children;
                                updated_children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
                                node.children = Some(updated_children);
                                node.is_expanded = true;
                            }
                        } else {
                            // Fallback for old protocol
                            let mut found = false;
                            Self::update_slave_node_static(&mut self.tree_explorer.slave_tree.root_nodes, children, &mut found);
                        }
                    }
                }
            }
            MasterEvent::RefreshTree { is_slave } => {
                if is_slave {
                    // For slave, we don't know the exact path easily from here, 
                    // so we refresh the whole tree or at least the drives if empty
                    if self.tree_explorer.slave_tree.root_nodes.is_empty() {
                        // This will be handled by the next draw or we could trigger it here
                    }
                    // Actually, the user can press F5 now. 
                    // To auto-refresh, we need to know the path.
                    // For now, let's just log that a refresh might be needed.
                    self.logs.push("Slave operation complete. Press F5 to refresh if changes not visible.".to_string());
                } else {
                    self.tree_refresh();
                }
            }
        }
    }

    fn update_slave_node_static(nodes: &mut Vec<FileNode>, children: Vec<FileNode>, found: &mut bool) {
        for node in nodes {
            if node.is_expanded && node.children.is_none() && node.is_dir {
                let mut updated_children = children.clone();
                for child in &mut updated_children {
                    let mut child_path = node.path.clone();
                    child_path.push(&child.name);
                    child.path = child_path;
                }
                updated_children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
                node.children = Some(updated_children);
                *found = true;
                return;
            }
            if let Some(children_vec) = &mut node.children {
                Self::update_slave_node_static(children_vec, children.clone(), found);
                if *found { return; }
            }
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let buf = frame.buffer_mut();

        // 1. Render Tab Bar (Top)
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);
        
        let tab_area = layout[0];
        let content_area = layout[1];

        let tab_titles = vec![" [F1] Main Console ", " [F2] Tree Explorer ", " [F3] System & Settings "];
        let tab_spans: Vec<Span> = tab_titles.iter().enumerate().map(|(i, title)| {
            let style = if (i == 0 && self.active_tab == Tab::Main) || 
                           (i == 1 && self.active_tab == Tab::TreeExplorer) ||
                           (i == 2 && self.active_tab == Tab::SystemSettings) {
                Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            Span::styled(*title, style)
        }).collect();

        Paragraph::new(Line::from(tab_spans))
            .block(Block::bordered().border_style(Style::default().fg(Color::DarkGray)))
            .render(tab_area, buf);

        // 2. Render Active Tab Content
        match self.active_tab {
            Tab::Main => self.render_main_tab(content_area, buf),
            Tab::TreeExplorer => self.render_tree_tab(content_area, buf),
            Tab::SystemSettings => self.render_system_tab(content_area, buf),
        }
    }

    fn render_system_tab(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title(Span::styled(" System Actions & Settings ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray));
        
        let inner = block.inner(area);
        block.render(area, buf);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10), // System Actions
                Constraint::Length(10), // Settings
                Constraint::Min(0),
            ])
            .split(inner);

        // --- System Actions ---
        let actions_block = Block::bordered()
            .title(Span::styled(" Remote System Actions ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray));
        let actions_inner = actions_block.inner(layout[0]);
        actions_block.render(layout[0], buf);

        let actions = vec![
            Line::from(vec![Span::styled("[1] Shutdown", Style::default().fg(Color::Red)), Span::raw(" - Power off the remote slave")]),
            Line::from(vec![Span::styled("[2] Reboot", Style::default().fg(Color::Yellow)), Span::raw(" - Restart the remote slave")]),
            Line::from(vec![Span::styled("[3] Sleep", Style::default().fg(Color::Blue)), Span::raw(" - Put remote slave to sleep")]),
            Line::from(vec![Span::styled("[4] Wake Up", Style::default().fg(Color::Green)), Span::raw(" - Send Wake-on-LAN (if supported)")]),
        ];
        Paragraph::new(actions).render(actions_inner, buf);

        // --- Settings ---
        let settings_block = Block::bordered()
            .title(Span::styled(" Deployment & Settings ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray));
        let settings_inner = settings_block.inner(layout[1]);
        settings_block.render(layout[1], buf);

        let settings = vec![
            Line::from(vec![Span::styled("[S] Install as System Service", Style::default().fg(Color::Gray)), Span::raw(" (Not implemented)")]),
            Line::from(vec![Span::styled("[A] Auto-start on boot", Style::default().fg(Color::Gray)), Span::raw(" (Not implemented)")]),
            Line::from(vec![Span::styled("[L] Log Level: ", Style::default().fg(Color::Gray)), Span::styled("INFO", Style::default().fg(Color::Green))]),
        ];
        Paragraph::new(settings).render(settings_inner, buf);
    }

    fn render_main_tab(&self, area: Rect, buf: &mut Buffer) {
        // Outer block
        let outer_block = Block::bordered()
            .title(Line::from(vec![
                Span::raw(" Tix-Master-V0.1---"),
                Span::styled("YuTech Labs", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
            ]).centered())
            .border_set(border::THICK)
            .border_style(Style::default().fg(Color::DarkGray));
        
        let inner_area = outer_block.inner(area);
        outer_block.render(area, buf);

        // Split inner area into Main (Top) and Input (Bottom)
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(inner_area);

        let top_area = main_layout[0];
        let input_area = main_layout[1];

        // Split Top area into Logs (Left) and Sidebar (Right)
        let top_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(70),
                Constraint::Percentage(30),
            ])
            .split(top_area);

        let logs_area = top_layout[0];
        let sidebar_area = top_layout[1];

        // --- Render Logs ---
        let logs_block = Block::bordered()
            .title(Line::from(vec![
                Span::styled(" Master Logs ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                if self.autoscroll {
                    Span::styled("[Autoscroll]", Style::default().fg(Color::Green).add_modifier(Modifier::DIM))
                } else {
                    Span::styled("[Manual]", Style::default().fg(Color::Yellow).add_modifier(Modifier::DIM))
                }
            ]))
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(ratatui::widgets::Padding::horizontal(1));
        
        let logs_inner = logs_block.inner(logs_area);
        logs_block.render(logs_area, buf);

        let visible_height = logs_inner.height as usize;
        let total_logs = self.logs.len();
        
        // Calculate which logs to show based on scroll
        let log_items: Vec<ListItem> = if total_logs <= visible_height {
            // If we have fewer logs than space, just show them all
            self.logs.iter()
        } else {
            // Calculate start index based on scroll from the bottom
            // scroll 0 = last `visible_height` logs
            let start = total_logs.saturating_sub(visible_height).saturating_sub(self.log_scroll);
            let end = (start + visible_height).min(total_logs);
            self.logs[start..end].iter()
        }
        .map(|log| {
            if log.starts_with(">") {
                ListItem::new(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Green)),
                    Span::raw(&log[2..]),
                ]))
            } else if log.starts_with("-") {
                ListItem::new(Line::from(vec![
                    Span::styled("- ", Style::default().fg(Color::Blue)),
                    Span::styled(&log[2..], Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)),
                ]))
            } else if log.starts_with("[SEND]") {
                ListItem::new(Line::from(vec![
                    Span::styled("→ ", Style::default().fg(Color::Cyan)),
                    Span::styled(log, Style::default().fg(Color::DarkGray)),
                ]))
            } else if log.starts_with("[RECV]") || log.starts_with("[DONE]") {
                ListItem::new(Line::from(vec![
                    Span::styled("← ", Style::default().fg(Color::Green)),
                    Span::styled(log, Style::default().fg(Color::DarkGray)),
                ]))
            } else if log.contains("stdout:") || log.contains("stderr:") {
                 // Format shell output lines specifically if needed, 
                 // but for now let's just clean them up
                 ListItem::new(Line::from(log.as_str()))
            } else {
                ListItem::new(Line::from(log.as_str()))
            }
        })
        .collect();
        
        let logs_list = List::new(log_items);
        logs_list.render(logs_inner, buf);

        // --- Render Sidebar ---
        let sidebar_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10), // Info box
                Constraint::Min(0),    // Tasks box
            ])
            .split(sidebar_area);

        let info_area = sidebar_layout[0];
        let tasks_area = sidebar_layout[1];

        // Info Box (Slave + Master)
        let info_block = Block::bordered()
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(ratatui::widgets::Padding::uniform(1));
        let info_inner = info_block.inner(info_area);
        info_block.render(info_area, buf);

        let mut info_text = vec![
            Line::from(vec![Span::styled("Slave PC :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
            Line::from(vec![
                Span::styled("IP    : ", Style::default().fg(Color::Gray)),
                Span::styled(&self.slave_info.ip, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![
                Span::styled("Ram   : ", Style::default().fg(Color::Gray)),
                Span::styled(&self.slave_info.ram_usage, Style::default().fg(Color::Magenta)),
            ]),
        ];
        for other in &self.slave_info.other {
            info_text.push(Line::from(vec![Span::styled(other, Style::default().fg(Color::DarkGray))]));
        }
        info_text.push(Line::from(""));
        info_text.push(Line::from(vec![Span::styled("Master PC (this):", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]));
        info_text.push(Line::from(vec![
            Span::styled("IP    : ", Style::default().fg(Color::Gray)),
            Span::styled(&self.master_info.ip, Style::default().fg(Color::Yellow)),
        ]));

        Paragraph::new(info_text).render(info_inner, buf);

        // Tasks Box
        let tasks_block = Block::bordered()
            .title(Span::styled(" Tasks : ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(ratatui::widgets::Padding::horizontal(1));
        let tasks_inner = tasks_block.inner(tasks_area);
        tasks_block.render(tasks_area, buf);

        let task_items: Vec<ListItem> = self.tasks.iter()
            .map(|task| {
                let color = if task.contains("Running") || task.contains("Solved") {
                    Color::Green
                } else if task.contains("Waiting") {
                    Color::Yellow
                } else if task.contains("Failed") {
                    Color::Red
                } else {
                    Color::Gray
                };
                ListItem::new(Line::from(vec![
                    Span::styled(task, Style::default().fg(color)),
                ]))
            })
            .collect();
        List::new(task_items).render(tasks_inner, buf);

        // --- Render Input ---
        let input_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray));
        let input_inner = input_block.inner(input_area);
        input_block.render(input_area, buf);

        let input_text = Line::from(vec![
            Span::styled(" > ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(&self.command_to_execute),
        ]);
        Paragraph::new(input_text).render(input_inner, buf);

        // --- Render Autocomplete Dropdown ---
        if self.completion.active && !self.completion.options.is_empty() {
            let num_options = self.completion.options.len().min(10);
            let dropdown_height = (num_options + 2) as u16;
            let dropdown_width = 40.min(inner_area.width - 4);
            
            // Position above the input bar
            let dropdown_area = Rect {
                x: input_inner.x + 3, // Offset by " > "
                y: input_area.y.saturating_sub(dropdown_height),
                width: dropdown_width,
                height: dropdown_height,
            };

            // Clear the area under the dropdown
            Clear.render(dropdown_area, buf);

            let dropdown_block = Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(" Suggestions ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
            
            let list_items: Vec<ListItem> = self.completion.options.iter().enumerate().map(|(i, opt)| {
                let style = if i == self.completion.selected_index {
                    Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let icon = if opt.is_dir {
                    Span::styled("📁 ", Style::default().fg(Color::Yellow))
                } else {
                    Span::styled("📄 ", Style::default().fg(Color::Blue))
                };

                ListItem::new(Line::from(vec![
                    icon,
                    Span::styled(&opt.display, style),
                ]))
            }).collect();

            let list = List::new(list_items)
                .block(dropdown_block)
                .highlight_symbol(">> ");
            
            list.render(dropdown_area, buf);
        }
    }

    fn render_tree_tab(&mut self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(60), // Tree views
                Constraint::Percentage(40), // Action bar
            ])
            .split(area);

        let tree_area = layout[0];
        let action_area = layout[1];

        // Split trees horizontally
        let tree_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(tree_area);

        // Local tree
        let active_side = self.tree_explorer.active_side;
        self.render_tree_panel(" Host Tree (Local) ", false, tree_layout[0], buf, !active_side);
        
        // Slave tree
        self.render_tree_panel(" Slave Tree (Remote) ", true, tree_layout[1], buf, active_side);

        self.render_action_bar(action_area, buf);
    }

    fn render_tree_panel(&mut self, title: &str, is_slave: bool, area: Rect, buf: &mut Buffer, is_active: bool) {
        let border_color = if is_active { Color::Cyan } else { Color::DarkGray };
        let block = Block::bordered()
            .title(Span::styled(title, Style::default().fg(border_color).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(border_color));
        
        let inner = block.inner(area);
        block.render(area, buf);

        let mut items = Vec::new();
        let (root_nodes, cursor_index, scroll_offset) = if !is_slave {
            (&self.tree_explorer.local_tree.root_nodes, self.tree_explorer.local_tree.cursor_index, &mut self.tree_explorer.local_tree.scroll_offset)
        } else {
            (&self.tree_explorer.slave_tree.root_nodes, self.tree_explorer.slave_tree.cursor_index, &mut self.tree_explorer.slave_tree.scroll_offset)
        };

        Self::flatten_tree_static(root_nodes, 0, &mut items);

        // Adjust scroll offset to follow cursor
        let height = inner.height as usize;
        if height > 0 {
            if cursor_index < *scroll_offset {
                *scroll_offset = cursor_index;
            } else if cursor_index >= *scroll_offset + height {
                *scroll_offset = cursor_index - height + 1;
            }
        }

        let list_items: Vec<ListItem> = items.iter().enumerate().skip(*scroll_offset).take(height).map(|(i, (node, depth))| {
            let indent = "  ".repeat(*depth);
            let icon = if node.is_dir {
                if node.is_expanded { "📂 " } else { "📁 " }
            } else {
                "📄 "
            };
            
            let selection_mark = if node.is_selected { "[x] " } else { "[ ] " };
            let style = if is_active && i == cursor_index {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::raw(indent),
                Span::styled(selection_mark, Style::default().fg(Color::Yellow)),
                Span::raw(icon),
                Span::styled(&node.name, style),
            ]))
        }).collect();

        List::new(list_items).render(inner, buf);
    }

    fn flatten_tree_static<'a>(nodes: &'a [FileNode], depth: usize, out: &mut Vec<(&'a FileNode, usize)>) {
        for node in nodes {
            out.push((node, depth));
            if node.is_expanded {
                if let Some(children) = &node.children {
                    Self::flatten_tree_static(children, depth + 1, out);
                }
            }
        }
    }

    fn render_action_bar(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title(Span::styled(" Actions ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray));
        
        let inner = block.inner(area);
        block.render(area, buf);

        let actions = vec![
            "[Space] Select",
            "[Enter] Open/Close",
            "[C] Copy",
            "[X] Cut",
            "[V] Paste",
            "[F5] Refresh",
            "[Del] Delete",
        ];

        let action_spans: Vec<Line> = actions.iter().map(|a| Line::from(Span::styled(*a, Style::default().fg(Color::Gray)))).collect();
        Paragraph::new(action_spans).render(inner, buf);
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // This is now redundant since we use Frame directly in draw(),
        // but kept for compatibility if needed.
    }
}



