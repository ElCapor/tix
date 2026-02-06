/*
##    ##    ###     ######  ######## ######## ####### 
###  ###   ## ##   ##    ##    ##    ##       ##    ##
########  ##   ##  ##          ##    ##       ##    ##
## ## ## ##     ##  ######     ##    ######   ####### 
##    ## #########       ##    ##    ##       ##  ##  
##    ## ##     ## ##    ##    ##    ##       ##   ## 
##    ## ##     ##  ######     ##    ######## ##    ##
*/


use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{backend::CrosstermBackend, Terminal};
use tix_master::{App, Master, MasterEvent, UiEvent};
use tix_core::ConnectionInfo;
use tokio::sync::mpsc;
use std::time::Duration;

#[tokio::main]
pub async fn main() -> std::io::Result<()> {
    // 1. Setup communication channels
    let (master_tx, mut master_rx) = mpsc::unbounded_channel::<MasterEvent>();
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiEvent>();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();

    // 2. Spawn Input Task (Dedicated thread for blocking crossterm poll)
    let input_ui_tx = ui_tx.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            if event::poll(Duration::from_millis(10)).unwrap_or(false) {
                if let Ok(event) = event::read() {
                    match event {
                        Event::Key(key) => {
                            if let Err(_) = input_ui_tx.send(UiEvent::Key(key)) {
                                break;
                            }
                        }
                        Event::Resize(w, h) => {
                            if let Err(_) = input_ui_tx.send(UiEvent::Resize(w, h)) {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    // 3. Spawn Master Task
    let master_event_tx = master_tx.clone();
    tokio::spawn(async move {
        let conn_info = ConnectionInfo::new("127.0.0.1".to_string(), 4321);
        let mut master = match Master::listen(conn_info, master_event_tx.clone()).await {
            Ok(m) => m,
            Err(e) => {
                let _ = master_event_tx.send(MasterEvent::Log(format!("Critical Error: Failed to start listener: {}", e)));
                return;
            }
        };

        loop {
            tokio::select! {
                // Handle commands from UI
                Some(cmd) = cmd_rx.recv() => {
                    if let Err(e) = master.execute_command(cmd).await {
                        let _ = master_event_tx.send(MasterEvent::Log(format!("Command Error: {}", e)));
                    }
                }
                
                // Handle network operations
                _ = async {
                    if !master.is_connected() {
                        let _ = master.accept_one().await;
                    } else {
                        let _ = master.process_connection().await;
                    }
                } => {}
            }
        }
    });

    // 4. Setup Terminal
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    let mut app = App::new();
    
    // 5. Main UI Event Loop (Purely Reactive)
    loop {
        terminal.draw(|f| app.draw(f))?;

        tokio::select! {
            // Handle Master events (Logs, Slave status, Task updates)
            Some(event) = master_rx.recv() => {
                app.update(event);
            }

            // Handle UI events (Keyboard, Resize)
            Some(event) = ui_rx.recv() => {
                match event {
                    UiEvent::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            match key.code {
                                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => break,
                                KeyCode::F(1) => app.set_tab(tix_master::Tab::Main),
                                KeyCode::F(2) => {
                                    app.set_tab(tix_master::Tab::TreeExplorer);
                                    if app.tree_explorer.slave_tree.root_nodes.is_empty() {
                                        if let Some(cmd) = app.refresh_slave_drives() {
                                            let _ = cmd_tx.send(cmd);
                                        }
                                    }
                                },
                                KeyCode::F(3) => app.set_tab(tix_master::Tab::SystemSettings),
                                KeyCode::Char('q') => app.exit = true,
                                KeyCode::Esc => app.handle_esc(),
                                
                                // Tab-specific navigation
                                KeyCode::Up if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_cursor_up(),
                                KeyCode::Down if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_cursor_down(),
                                KeyCode::Left if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_switch_side(),
                                KeyCode::Right if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_switch_side(),
                                KeyCode::Enter if app.active_tab == tix_master::Tab::TreeExplorer => {
                                    if let Some(cmd) = app.tree_toggle_expand() {
                                        let _ = cmd_tx.send(cmd);
                                    }
                                }
                                KeyCode::F(5) if app.active_tab == tix_master::Tab::TreeExplorer => {
                                    if let Some(cmd) = app.tree_refresh() {
                                        let _ = cmd_tx.send(cmd);
                                    }
                                }
                                KeyCode::Char(' ') if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_toggle_select(),
                                KeyCode::Char('c') if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_copy(),
                                KeyCode::Char('x') if app.active_tab == tix_master::Tab::TreeExplorer => app.tree_cut(),
                                KeyCode::Char('v') if app.active_tab == tix_master::Tab::TreeExplorer => {
                                    for cmd in app.tree_paste() {
                                        let _ = cmd_tx.send(cmd);
                                    }
                                }

                                // System tab actions
                                KeyCode::Char('1') if app.active_tab == tix_master::Tab::SystemSettings => {
                                    let _ = cmd_tx.send("SystemAction shutdown".to_string());
                                }
                                KeyCode::Char('2') if app.active_tab == tix_master::Tab::SystemSettings => {
                                    let _ = cmd_tx.send("SystemAction reboot".to_string());
                                }
                                KeyCode::Char('3') if app.active_tab == tix_master::Tab::SystemSettings => {
                                    let _ = cmd_tx.send("SystemAction sleep".to_string());
                                }

                                // Main tab console inputs
                                KeyCode::Tab if app.active_tab == tix_master::Tab::Main => app.handle_tab(),
                                KeyCode::Char(c) if app.active_tab == tix_master::Tab::Main => {
                                    app.command_to_execute.push(c);
                                    app.on_input_change();
                                }
                                KeyCode::Backspace if app.active_tab == tix_master::Tab::Main => { 
                                    app.command_to_execute.pop(); 
                                    app.on_input_change();
                                },
                                KeyCode::Up if app.active_tab == tix_master::Tab::Main => app.handle_up(),
                                KeyCode::Down if app.active_tab == tix_master::Tab::Main => app.handle_down(),
                                KeyCode::PageUp if app.active_tab == tix_master::Tab::Main => {
                                    app.log_scroll = (app.log_scroll + 10).min(app.logs.len().saturating_sub(1));
                                    app.autoscroll = false;
                                }
                                KeyCode::PageDown if app.active_tab == tix_master::Tab::Main => {
                                    app.log_scroll = app.log_scroll.saturating_sub(10);
                                    if app.log_scroll == 0 {
                                        app.autoscroll = true;
                                    }
                                }
                                KeyCode::Enter if app.active_tab == tix_master::Tab::Main => {
                                    if let Some(cmd) = app.handle_enter() {
                                        app.logs.push(format!("> {}", cmd));
                                        // Send command to Master task
                                        let _ = cmd_tx.send(cmd);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    UiEvent::Resize(_, _) => {
                        // Ratatui handles resize automatically on draw, 
                        // but we can trigger a redraw if we want.
                    }
                }
            }

            // Optional: Heartbeat or periodic UI tasks
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Debounced completion update
                if app.needs_completion_update && app.last_input_time.elapsed() >= Duration::from_millis(150) {
                    app.update_completion();
                }
            }
        }

        if app.exit {
            break;
        }
    }

    // Restore terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    
    Ok(())
}
