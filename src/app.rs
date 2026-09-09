use crate::{
    clipboard::{ClipboardEvent, ClipboardWorker},
    model::ClipItem,
    store,
};
use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Sense, Stroke,
    Vec2,
};
use std::{fs, time::Duration};

const MAX_HISTORY: usize = 500;
const PANEL: Color32 = Color32::from_rgb(27, 26, 31);
const PANEL_ALT: Color32 = Color32::from_rgb(31, 30, 36);
const SELECTED: Color32 = Color32::from_rgb(54, 53, 59);
const MUTED: Color32 = Color32::from_rgb(160, 157, 164);
const BORDER: Color32 = Color32::from_rgb(73, 71, 78);

pub struct ClipboardApp {
    items: Vec<ClipItem>,
    next_id: u64,
    query: String,
    selected_id: Option<u64>,
    worker: ClipboardWorker,
    status: Option<String>,
    dirty: bool,
    focus_search: bool,
    place_window: bool,
}

impl ClipboardApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);

        let items = store::load();
        let selected_id = items.first().map(|item| item.id);
        let next_id = items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        Self {
            items,
            next_id,
            query: String::new(),
            selected_id,
            worker: ClipboardWorker::start(),
            status: None,
            dirty: false,
            focus_search: true,
            place_window: true,
        }
    }

    fn receive_clipboard(&mut self) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                ClipboardEvent::Text(text) => self.push(text),
                ClipboardEvent::Unavailable(error) => {
                    self.status = Some(format!("Clipboard unavailable: {error}"));
                }
            }
        }
    }

    fn push(&mut self, text: String) {
        if let Some(position) = self.items.iter().position(|item| item.text == text) {
            let mut existing = self.items.remove(position);
            existing.created_at = crate::model::now_secs();
            self.items.insert(0, existing);
        } else {
            self.items.insert(0, ClipItem::new(self.next_id, text));
            self.next_id += 1;
        }
        if self.selected_id.is_none() {
            self.selected_id = self.items.first().map(|item| item.id);
        }
        self.trim_history();
        self.dirty = true;
    }

    fn trim_history(&mut self) {
        if self.items.len() <= MAX_HISTORY {
            return;
        }
        let mut index = self.items.len();
        while self.items.len() > MAX_HISTORY && index > 0 {
            index -= 1;
            if !self.items[index].favorite {
                self.items.remove(index);
            }
        }
    }

    fn visible_items(&self) -> Vec<ClipItem> {
        let query = self.query.to_lowercase();
        self.items
            .iter()
            .filter(|item| query.is_empty() || item.text.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn move_selection(&mut self, visible: &[ClipItem], delta: isize) {
        if visible.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .and_then(|id| visible.iter().position(|item| item.id == id))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
        self.selected_id = Some(visible[next].id);
    }

    fn copy_selected(&mut self) {
        let Some(item) = self
            .selected_id
            .and_then(|id| self.items.iter().find(|item| item.id == id))
        else {
            return;
        };
        self.worker.set_text(item.text.clone());
        self.status = Some("Copied to clipboard".to_owned());
    }

    fn delete_selected(&mut self) {
        let Some(id) = self.selected_id else {
            return;
        };
        let position = self.items.iter().position(|item| item.id == id);
        self.items.retain(|item| item.id != id);
        self.selected_id = position
            .and_then(|index| self.items.get(index).or_else(|| self.items.last()))
            .map(|item| item.id);
        self.dirty = true;
    }

    fn toggle_favorite(&mut self) {
        if let Some(item) = self
            .selected_id
            .and_then(|id| self.items.iter_mut().find(|item| item.id == id))
        {
            item.favorite = !item.favorite;
            self.dirty = true;
        }
    }

    fn persist_if_needed(&mut self) {
        if !self.dirty {
            return;
        }
        match store::save(&self.items) {
            Ok(()) => self.dirty = false,
            Err(error) => self.status = Some(format!("Could not save history: {error}")),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let drag = ui.allocate_response(Vec2::new(ui.available_width(), 8.0), Sense::drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.horizontal(|ui| {
            ui.add_space(14.0);
            if ui
                .add(egui::Button::new(RichText::new("‹").size(34.0)).frame(false))
                .on_hover_text("Close")
                .clicked()
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(8.0);

            let search_width = (ui.available_width() - 170.0).max(200.0);
            let response = ui.add_sized(
                [search_width, 48.0],
                egui::TextEdit::singleline(&mut self.query)
                    .id(egui::Id::new("search"))
                    .font(FontId::proportional(25.0))
                    .frame(egui::Frame::NONE)
                    .hint_text("Type to filter entries…"),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("Text").size(17.0).color(MUTED));
            });
        });
        ui.add_space(9.0);
    }

    fn history_list(&mut self, ui: &mut egui::Ui, visible: &[ClipItem]) -> bool {
        let mut copy = false;
        ui.add_space(10.0);
        ui.label(RichText::new("Today").size(14.0).strong().color(MUTED));
        ui.add_space(7.0);

        if visible.is_empty() {
            ui.add_space(70.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No matching clips").size(17.0).color(MUTED));
            });
            return false;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in visible {
                    let selected = self.selected_id == Some(item.id);
                    let fill = if selected {
                        SELECTED
                    } else {
                        Color32::TRANSPARENT
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
                                        ui.label(RichText::new("T").size(18.0).strong());
                                    });
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.add_space(2.0);
                                    ui.label(RichText::new(item.preview(46)).size(16.0).strong());
                                    ui.label(
                                        RichText::new(item.age_label()).size(11.0).color(MUTED),
                                    );
                                });
                                if item.favorite {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(RichText::new("★").color(Color32::YELLOW));
                                        },
                                    );
                                }
                            });
                        });
                    let response = row.response.interact(Sense::click());
                    if response.clicked() {
                        self.selected_id = Some(item.id);
                    }
                    if response.double_clicked() {
                        self.selected_id = Some(item.id);
                        copy = true;
                    }
                    ui.add_space(4.0);
                }
            });
        copy
    }

    fn details(ui: &mut egui::Ui, selected: Option<&ClipItem>) {
        ui.add_space(20.0);
        let Some(item) = selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(90.0);
                ui.label(RichText::new("Select an entry").size(18.0).color(MUTED));
            });
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new(&item.text).size(18.0).strong());
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(18.0);
            detail_row(ui, "Copied", &item.age_label());
            detail_row(
                ui,
                "Words",
                &item.text.split_whitespace().count().to_string(),
            );
            detail_row(ui, "Characters", &item.text.chars().count().to_string());
            detail_row(ui, "Type", "Text");
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
                RichText::new(self.status.as_deref().unwrap_or("Clipboard History"))
                    .size(16.0)
                    .strong(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .button("Delete")
                    .on_hover_text("Command + Backspace")
                    .clicked()
                {
                    self.delete_selected();
                }
                if ui.button("Favorite").on_hover_text("Command + S").clicked() {
                    self.toggle_favorite();
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("Copy   Enter").strong()).corner_radius(18),
                    )
                    .clicked()
                {
                    self.copy_selected();
                }
            });
        });
        ui.add_space(9.0);
    }
}

impl eframe::App for ClipboardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_clipboard();
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(250));

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

        let visible = self.visible_items();
        if self
            .selected_id
            .is_none_or(|id| !visible.iter().any(|item| item.id == id))
        {
            self.selected_id = visible.first().map(|item| item.id);
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            self.move_selection(&visible, 1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            self.move_selection(&visible, -1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.copy_selected();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.toggle_favorite();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace))
        {
            self.delete_selected();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(22)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                self.header(ui);
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
                                if self.history_list(ui, &visible) {
                                    self.copy_selected();
                                }
                            },
                        );
                        ui.add(egui::Separator::default().vertical().grow(0.0));
                        let detail_width = ui.available_width();
                        let selected = self
                            .selected_id
                            .and_then(|id| visible.iter().find(|item| item.id == id));
                        ui.allocate_ui_with_layout(
                            Vec2::new(detail_width, content_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(detail_width);
                                ui.add_space(18.0);
                                ui.indent("detail", |ui| Self::details(ui, selected));
                            },
                        );
                    },
                );
                ui.separator();
                self.footer(ui);
            });

        self.persist_if_needed();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = store::save(&self.items);
    }
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
                    ui.label(RichText::new(value).size(14.0).strong());
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
