//! Console of the editor

use super::EditorState;
use dear_imgui_rs::{Condition, StyleColor, Ui};

pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Default)]
pub struct ConsoleLogger {
    pub entries: Vec<LogEntry>,
    pub scroll_to_bottom: bool,
}

impl ConsoleLogger {
    pub fn info(&mut self, msg: impl Into<String>) {
        let text = msg.into();
        println!("    {}", &text);
        self.entries.push(LogEntry {
            level: LogLevel::Info,
            text,
        });
        self.scroll_to_bottom = true;
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let text = msg.into();
        println!("[W] {}", &text);
        self.entries.push(LogEntry {
            level: LogLevel::Warn,
            text,
        });
        self.scroll_to_bottom = true;
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        let text = msg.into();
        eprintln!("[E] {}", &text);
        self.entries.push(LogEntry {
            level: LogLevel::Error,
            text,
        });
        self.scroll_to_bottom = true;
    }
}

pub fn draw_console(ui: &Ui, state: &mut EditorState) {
    ui.window("Console")
        .size([1280.0, 180.0], Condition::FirstUseEver)
        .build(|| {
            if ui.small_button("Clear") {
                state.console.entries.clear();
            }
            ui.same_line();
            ui.text("Filter:");
            ui.same_line();
            ui.set_next_item_width(180.0);
            let mut fbuf = state.con_filter.clone();
            if ui
                .input_text("##con_filter", &mut fbuf)
                .hint("search…")
                .build()
            {
                state.con_filter = fbuf;
            }

            ui.separator();

            ui.child_window("##console_log")
                .size(ui.content_region_avail())
                .build(ui, || {
                    let filter_lc = state.con_filter.to_ascii_lowercase();

                    for entry in &state.console.entries {
                        if !filter_lc.is_empty()
                            && !entry.text.to_ascii_lowercase().contains(&filter_lc)
                        {
                            continue;
                        }

                        let col = match entry.level {
                            LogLevel::Info => state.core.palette.console_info,
                            LogLevel::Warn => state.core.palette.console_warn,
                            LogLevel::Error => state.core.palette.console_error,
                        };

                        let _tok = ui.push_style_color(StyleColor::Text, col);
                        ui.text(format!("{}", entry.text));
                    }

                    if state.console.scroll_to_bottom {
                        ui.set_scroll_here_y(1.0);
                        state.console.scroll_to_bottom = false;
                    }
                });
        });
}
