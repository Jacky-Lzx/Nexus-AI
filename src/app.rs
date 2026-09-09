use crate::hotkey::{HotkeyController, SHORTCUT_LABEL};
use clipkeep_core::SearchCore;
use clipkeep_protocol::{ResultKind, SearchResult};
use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Sense, Stroke,
    Vec2,
};
use std::{fs, time::Duration};

const PANEL: Color32 = Color32::from_rgb(27, 26, 31);
const PANEL_ALT: Color32 = Color32::from_rgb(31, 30, 36);
const SELECTED: Color32 = Color32::from_rgb(54, 53, 59);
const MUTED: Color32 = Color32::from_rgb(160, 157, 164);
const BORDER: Color32 = Color32::from_rgb(73, 71, 78);

pub struct LauncherApp {
    core: Option<SearchCore>,
    items: Vec<SearchResult>,
    query: String,
    selected_id: Option<String>,
    status: Option<String>,
    focus_search: bool,
    place_window: bool,
    hotkey: Option<HotkeyController>,
    visible: bool,
    had_focus: bool,
    hide_after_action: bool,
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);
        let (core, mut status) = match SearchCore::with_builtin_plugins() {
            Ok(core) => (Some(core), None),
            Err(error) => (None, Some(format!("Could not start plugins: {error}"))),
        };
        let hotkey = match HotkeyController::register(cc.egui_ctx.clone()) {
            Ok(hotkey) => Some(hotkey),
            Err(error) => {
                status = Some(format!("Could not register {SHORTCUT_LABEL}: {error}"));
                None
            }
        };
        Self {
            core,
            items: Vec::new(),
            query: String::new(),
            selected_id: None,
            status,
            focus_search: true,
            place_window: true,
            hotkey,
            visible: true,
            had_focus: false,
            hide_after_action: false,
        }
    }

    fn poll_core(&mut self) {
        let Some(core) = &mut self.core else {
            return;
        };
        if core.poll() {
            self.items = core.results().to_vec();
        }
        if let Some(status) = core.take_status() {
            self.status = Some(status);
        }
    }

    fn search(&mut self) {
        if let Some(core) = &mut self.core {
            core.search(self.query.clone());
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .as_ref()
            .and_then(|id| self.items.iter().position(|item| &item.id == id))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, self.items.len() as isize - 1) as usize;
        self.selected_id = Some(self.items[next].id.clone());
    }

    fn selected(&self) -> Option<SearchResult> {
        self.selected_id
            .as_ref()
            .and_then(|id| self.items.iter().find(|item| &item.id == id))
            .cloned()
    }

    fn execute_selected(&mut self, action_id: &str) {
        let Some(result) = self.selected() else {
            return;
        };
        if let Some(core) = &mut self.core {
            core.execute(&result, action_id);
        }
    }

    fn primary_action(&mut self) {
        let action = match self.selected().map(|result| result.kind) {
            Some(ResultKind::Application) => "open",
            Some(ResultKind::ClipboardText) => "copy",
            _ => return,
        };
        self.execute_selected(action);
        self.hide_after_action = true;
    }

    fn hide(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.visible = false;
        self.had_focus = false;
        self.hide_after_action = false;
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.visible = true;
        self.had_focus = false;
        self.focus_search = true;
        self.place_window = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn toggle_visibility(&mut self, ctx: &egui::Context) {
        if self.visible {
            self.hide(ctx);
        } else {
            self.show(ctx);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) -> bool {
        let drag = ui.allocate_response(Vec2::new(ui.available_width(), 8.0), Sense::drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            if ui
                .add(egui::Button::new(RichText::new("‹").size(34.0)).frame(false))
                .on_hover_text("Close")
                .clicked()
            {
                self.hide(ui.ctx());
            }
            ui.add_space(8.0);
            let search_width = (ui.available_width() - 170.0).max(200.0);
            let response = ui.add_sized(
                [search_width, 48.0],
                egui::TextEdit::singleline(&mut self.query)
                    .id(egui::Id::new("search"))
                    .font(FontId::proportional(25.0))
                    .frame(egui::Frame::NONE)
                    .hint_text("Search applications and clipboard…"),
            );
            changed = response.changed();
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);
                ui.menu_button(RichText::new("Plugins").size(17.0).color(MUTED), |ui| {
                    let states = self
                        .core
                        .as_ref()
                        .map(SearchCore::plugin_states)
                        .unwrap_or_default();
                    for state in states {
                        let mut enabled = state.enabled;
                        if ui.checkbox(&mut enabled, &state.name).changed()
                            && let Some(core) = &mut self.core
                            && let Err(error) = core.set_plugin_enabled(&state.id, enabled)
                        {
                            self.status = Some(format!("Could not update plugin: {error}"));
                        }
                    }
                });
            });
        });
        ui.add_space(9.0);
        changed
    }

    fn result_list(&mut self, ui: &mut egui::Ui) -> bool {
        let mut execute = false;
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("Results · {}", self.items.len()))
                .size(14.0)
                .strong()
                .color(MUTED),
        );
        ui.add_space(7.0);

        if self.items.is_empty() {
            ui.add_space(70.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No results").size(17.0).color(MUTED));
            });
            return false;
        }

        let items = self.items.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in &items {
                    let selected = self.selected_id.as_ref() == Some(&item.id);
                    let fill = if selected {
                        SELECTED
                    } else {
                        Color32::TRANSPARENT
                    };
                    let icon = match item.kind {
                        ResultKind::Application => "A",
                        ResultKind::ClipboardText => "T",
                        ResultKind::Command => ">",
                        ResultKind::File => "F",
                        ResultKind::Other => "·",
                    };
                    let row = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(12)
                        .inner_margin(Margin::symmetric(12, 11))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(59, 58, 65))
                                    .corner_radius(8)
                                    .inner_margin(7)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(icon).size(17.0).strong());
                                    });
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.add_space(2.0);
                                    ui.label(RichText::new(&item.title).size(16.0).strong());
                                    ui.label(RichText::new(&item.subtitle).size(11.0).color(MUTED));
                                });
                                if item
                                    .metadata
                                    .get("favorite")
                                    .is_some_and(|value| value == "true")
                                {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(RichText::new("*").color(Color32::YELLOW));
                                        },
                                    );
                                }
                            });
                        });
                    let response = row.response.interact(Sense::click());
                    if response.clicked() {
                        self.selected_id = Some(item.id.clone());
                    }
                    if response.double_clicked() {
                        self.selected_id = Some(item.id.clone());
                        execute = true;
                    }
                    ui.add_space(4.0);
                }
            });
        execute
    }

    fn details(ui: &mut egui::Ui, selected: Option<&SearchResult>) {
        ui.add_space(20.0);
        let Some(item) = selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(90.0);
                ui.label(RichText::new("Select a result").size(18.0).color(MUTED));
            });
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new(&item.preview).size(18.0).strong());
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(18.0);
            for (key, value) in item.metadata.iter().rev() {
                detail_row(ui, &title_case(key), value);
            }
            detail_row(ui, "Plugin", &item.plugin_id);
            detail_row(ui, "Type", result_kind_name(item.kind));
            ui.add_space(8.0);
            ui.label(
                RichText::new("Information")
                    .size(14.0)
                    .strong()
                    .color(MUTED),
            );
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected();
        let primary_label = match selected.as_ref().map(|result| result.kind) {
            Some(ResultKind::Application) => "Open   Enter",
            Some(ResultKind::ClipboardText) => "Copy   Enter",
            _ => "Run   Enter",
        };
        ui.add_space(9.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("C")
                    .size(23.0)
                    .strong()
                    .color(Color32::from_rgb(255, 109, 45)),
            );
            ui.label(
                RichText::new(self.status.as_deref().unwrap_or("ClipKeep Launcher"))
                    .size(16.0)
                    .strong(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if ui.small_button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if selected
                    .as_ref()
                    .is_some_and(|result| result.kind == ResultKind::ClipboardText)
                {
                    if ui
                        .button("Delete")
                        .on_hover_text("Command + Backspace")
                        .clicked()
                    {
                        self.execute_selected("delete");
                    }
                    if ui.button("Favorite").on_hover_text("Command + S").clicked() {
                        self.execute_selected("favorite");
                    }
                }
                if ui
                    .add(egui::Button::new(RichText::new(primary_label).strong()).corner_radius(18))
                    .clicked()
                {
                    self.primary_action();
                }
                ui.label(RichText::new(SHORTCUT_LABEL).size(12.0).color(MUTED));
            });
        });
        ui.add_space(9.0);
    }
}

impl eframe::App for LauncherApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_core();
        if self
            .hotkey
            .as_ref()
            .is_some_and(HotkeyController::take_triggered)
        {
            self.toggle_visibility(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(50));

        if !self.visible {
            return;
        }

        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(false));
        if focused {
            self.had_focus = true;
        } else if self.had_focus {
            self.hide(&ctx);
            return;
        }

        if self.place_window
            && let Some(monitor) = ctx.input(|input| input.viewport().monitor_size)
        {
            let size = ui.available_size();
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                ((monitor.x - size.x) / 2.0).max(0.0),
                ((monitor.y - size.y) / 2.0).max(0.0),
            )));
            self.place_window = false;
        }

        if self
            .selected_id
            .as_ref()
            .is_none_or(|id| !self.items.iter().any(|item| &item.id == id))
        {
            self.selected_id = self.items.first().map(|item| item.id.clone());
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            self.move_selection(1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            self.move_selection(-1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.primary_action();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.execute_selected("favorite");
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace))
        {
            self.execute_selected("delete");
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.hide(&ctx);
            return;
        }

        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(22)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if self.header(ui) {
                    self.search();
                }
                ui.separator();

                let footer_height = 56.0;
                let content_height = (ui.available_height() - footer_height).max(240.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), content_height),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        let left_width = (ui.available_width() * 0.38).clamp(270.0, 390.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(left_width, content_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(left_width);
                                if self.result_list(ui) {
                                    self.primary_action();
                                }
                            },
                        );
                        ui.add(egui::Separator::default().vertical().grow(0.0));
                        let detail_width = ui.available_width();
                        let selected = self.selected();
                        ui.allocate_ui_with_layout(
                            Vec2::new(detail_width, content_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(detail_width);
                                ui.add_space(18.0);
                                ui.indent("detail", |ui| Self::details(ui, selected.as_ref()));
                            },
                        );
                    },
                );
                ui.separator();
                self.footer(ui);
            });

        if self.hide_after_action {
            self.hide(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn result_kind_name(kind: ResultKind) -> &'static str {
    match kind {
        ResultKind::Application => "Application",
        ResultKind::ClipboardText => "Clipboard Text",
        ResultKind::Command => "Command",
        ResultKind::File => "File",
        ResultKind::Other => "Other",
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new()
        .fill(PANEL_ALT)
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(14.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(value).size(14.0).strong()).truncate());
                });
            });
        });
    ui.add_space(2.0);
}

fn configure_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = PANEL;
    style.visuals.selection.bg_fill = SELECTED;
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(47, 46, 52);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(47, 46, 52);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(65, 63, 70);
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 8.0);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn configure_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("system-cjk".to_owned(), FontData::from_owned(bytes).into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}
