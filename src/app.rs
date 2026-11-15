//! Console dashboard, registration, settings, logs, and the streaming view.

use crate::config::{Config, SavedHost, decode_account_id};
use crate::input::{self, Input};
use crate::logo::Logo;
use crate::remote::{self, Discovery, Registration, Session, SessionStatus, StreamOptions};
use crate::theme::{
    self, ACCENT, BG, BORDER, Icon, LIVE, PANEL, POPOVER, STANDBY, TEXT, TEXT_DIM, white,
};
use crate::video::VideoRenderer;
use eframe::egui::{self, Color32, CornerRadius, Key, Margin, RichText, Stroke, StrokeKind, Vec2};
use std::time::Duration;

const RAIL_WIDTH: f32 = 64.0;
const STREAM_TOOLBAR_HEIGHT: f32 = 58.0;
const PSN_ACCOUNT_GUIDE: &str =
    "https://chiaki-ng.github.io/chiaki-ng/setup/configuration/#registering-your-playstation";
const PSN_ID_FINDER: &str = "https://pyrosoft.pro/psnid/";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Consoles,
    Settings,
    Log,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Account,
    Video,
    Playback,
}

struct RegisterDialog {
    addr: String,
    ps5: bool,
    /// Opened from "Add console": the address is editable.
    manual: bool,
    watch_only: bool,
    expected_mac: Option<String>,
    account_input: String,
    focus_first: bool,
    pin: String,
    running: Option<(Registration, [u8; 8])>,
    error: Option<String>,
}

struct Streaming {
    session: Session,
    title: String,
    pin_input: String,
    menu_open: bool,
    watch_only: bool,
    using_second_account: bool,
    unsupported_format_logged: bool,
}

pub struct App {
    config: Config,
    discovery: Option<Discovery>,
    video: VideoRenderer,
    input: Input,
    page: Page,
    settings_section: SettingsSection,
    register: Option<RegisterDialog>,
    account_guide_open: bool,
    streaming: Option<Streaming>,
    banner: Option<(String, bool)>, // (message, is_error)
    fullscreen: bool,
    /// Animated B-snap mark at the top of the rail.
    logo: Logo,
    /// Console displayed in the details panel, keyed by `HostRow::key`.
    selected: Option<String>,
}

/// One console: what discovery sees merged with what we saved.
struct HostRow {
    name: String,
    addr: String,
    ps5: bool,
    awake: bool,
    standby: bool,
    seen: bool,
    running_app: String,
    saved: Option<SavedHost>,
    watch_saved: Option<SavedHost>,
}

impl HostRow {
    /// Stable identity: the console MAC once linked, otherwise its address.
    fn key(&self) -> String {
        self.link()
            .map_or_else(|| self.addr.clone(), |s| s.mac.clone())
    }

    fn link(&self) -> Option<&SavedHost> {
        self.saved.as_ref().or(self.watch_saved.as_ref())
    }

    fn saved_for(&self, watch_only: bool) -> Option<&SavedHost> {
        if watch_only {
            self.watch_saved.as_ref()
        } else {
            self.saved.as_ref()
        }
    }

    fn stream_link(&self, watch_only: bool) -> Option<&SavedHost> {
        self.saved_for(watch_only).or(self.saved.as_ref())
    }

    fn link_dialog(&self, second_account: bool) -> RegisterDialog {
        let mut dialog = new_register(self.addr.clone(), self.ps5, false, second_account);
        dialog.expected_mac = self.link().map(|saved| saved.mac.clone());
        dialog
    }

    fn display_name(&self, index: usize, streamer_mode: bool) -> String {
        if streamer_mode {
            format!("Console {}", index + 1)
        } else {
            self.name.clone()
        }
    }

    fn model(&self) -> &'static str {
        if self.ps5 {
            "PlayStation 5"
        } else {
            "PlayStation 4"
        }
    }

    fn status(&self) -> (&'static str, Option<Color32>) {
        if self.awake {
            ("Online", Some(LIVE))
        } else if self.standby {
            ("Rest mode", Some(STANDBY))
        } else {
            ("Offline", None)
        }
    }
}

enum HostAction {
    Play,
    Watch,
    Wake,
    Forget,
    ForgetWatchAccount,
    Register,
    RegisterWatchAccount,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        theme::apply(&cc.egui_ctx);
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the wgpu renderer is required"))?;
        let discovery = match Discovery::start(cc.egui_ctx.clone()) {
            Ok(d) => Some(d),
            Err(e) => {
                remote::log_line(format!("[E] {e:#}"));
                None
            }
        };
        let (config, settings_warning) = Config::load();
        if let Some(warning) = &settings_warning {
            remote::log_line(format!("[E] {warning}"));
        }
        Ok(Self {
            config,
            discovery,
            video: VideoRenderer::new(rs),
            input: Input::new(),
            page: Page::Consoles,
            settings_section: SettingsSection::Account,
            register: None,
            account_guide_open: false,
            streaming: None,
            banner: settings_warning.map(|warning| (warning, true)),
            fullscreen: false,
            logo: Logo::new(),
            selected: None,
        })
    }

    fn rows(&self) -> Vec<HostRow> {
        let discovered = self
            .discovery
            .as_ref()
            .map(|d| d.hosts())
            .unwrap_or_default();
        let mut rows: Vec<HostRow> = discovered
            .iter()
            .map(|d| HostRow {
                name: d.name.clone(),
                addr: d.addr.clone(),
                ps5: d.ps5,
                awake: d.awake,
                standby: d.standby,
                seen: true,
                running_app: d.running_app.clone(),
                saved: self
                    .config
                    .hosts
                    .iter()
                    .find(|h| h.mac == d.host_id)
                    .cloned(),
                watch_saved: self
                    .config
                    .watch_hosts
                    .iter()
                    .find(|h| h.mac == d.host_id)
                    .cloned(),
            })
            .collect();
        for saved in self.config.hosts.iter().chain(&self.config.watch_hosts) {
            if !rows.iter().any(|r| r.key() == saved.mac) {
                rows.push(HostRow {
                    name: saved.nickname.clone(),
                    addr: saved.last_addr.clone(),
                    ps5: saved.ps5,
                    awake: false,
                    standby: false,
                    seen: false,
                    running_app: String::new(),
                    saved: self
                        .config
                        .hosts
                        .iter()
                        .find(|h| h.mac == saved.mac)
                        .cloned(),
                    watch_saved: self
                        .config
                        .watch_hosts
                        .iter()
                        .find(|h| h.mac == saved.mac)
                        .cloned(),
                });
            }
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("BAKERY_PS_DEMO").is_some() {
            // Sample consoles only, so screenshots never show real ones.
            return demo_rows();
        }
        rows
    }

    fn connect(&mut self, ctx: &egui::Context, row: &HostRow, watch_only: bool) {
        let Some(saved) = row.stream_link(watch_only) else {
            self.register = Some(row.link_dialog(false));
            return;
        };
        let using_second_account = watch_only && row.watch_saved.is_some();
        let account = if using_second_account {
            saved
                .account_id
                .ok_or_else(|| {
                    "Set up the second account again to use it for Watch only.".to_owned()
                })
                .and_then(|account| {
                    self.config
                        .validate_watch_account(account, row.saved.as_ref())
                        .map(|()| account)
                })
        } else {
            saved
                .account_id
                .map_or_else(|| self.config.settings.account_id_bytes(), Ok)
        };
        let account = match account {
            Ok(a) => a,
            Err(e) => {
                if using_second_account {
                    let mut dialog = row.link_dialog(true);
                    dialog.error = Some(e);
                    self.register = Some(dialog);
                } else {
                    let mut dialog = row.link_dialog(false);
                    dialog.error = Some(format!("{e}. Link the console again to save it."));
                    self.register = Some(dialog);
                }
                return;
            }
        };
        let Some(host) = saved.to_registered() else {
            self.banner = Some((
                "Saved keys for this console are damaged; link it again.".into(),
                true,
            ));
            return;
        };
        let s = &self.config.settings;
        let opts = StreamOptions {
            resolution: s.resolution,
            fps: s.fps,
            codec: s.codec,
            bitrate_kbps: s.bitrate_kbps,
            hw_decode: s.hw_decode,
            volume_percent: s.volume_percent,
        };
        match Session::start(ctx.clone(), &row.addr, &host, &account, &opts) {
            Ok(session) => {
                // Remember the address in case discovery can't see the console next time.
                for watch in [false, true] {
                    if let Some(link) = self
                        .config
                        .hosts_mut(watch)
                        .iter_mut()
                        .find(|h| h.mac == saved.mac)
                    {
                        link.last_addr = row.addr.clone();
                    }
                }
                self.config.save();
                self.video.clear();
                self.streaming = Some(Streaming {
                    session,
                    title: row.name.clone(),
                    pin_input: String::new(),
                    menu_open: false,
                    watch_only,
                    using_second_account,
                    unsupported_format_logged: false,
                });
                self.banner = None;
                if self.config.settings.fullscreen_on_connect {
                    self.set_fullscreen(ctx, true);
                }
            }
            Err(e) => self.banner = Some((format!("{e:#}"), true)),
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn do_action(&mut self, ctx: &egui::Context, row: &HostRow, action: HostAction) {
        match action {
            HostAction::Play => self.connect(ctx, row, false),
            HostAction::Watch => self.connect(ctx, row, true),
            HostAction::Wake => {
                let saved = row.link().and_then(|s| s.to_registered());
                match saved.map(|h| remote::wakeup(&row.addr, &h.regist_key, row.ps5)) {
                    Some(Ok(())) => {
                        self.banner = Some((
                            format!("Wake-up sent to {}. It takes about 20 seconds.", row.name),
                            false,
                        ))
                    }
                    Some(Err(e)) => self.banner = Some((format!("{e:#}"), true)),
                    None => {}
                }
            }
            HostAction::Forget => {
                let key = row.key();
                for watch in [false, true] {
                    self.config.hosts_mut(watch).retain(|h| h.mac != key);
                }
                self.config.save();
            }
            HostAction::ForgetWatchAccount => {
                let key = row.key();
                self.config.hosts_mut(true).retain(|h| h.mac != key);
                self.config.save();
            }
            HostAction::Register => self.register = Some(row.link_dialog(false)),
            HostAction::RegisterWatchAccount => self.register = Some(row.link_dialog(true)),
        }
    }

    // ------------------------------------------------------------- home screen

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rows = self.rows();

        egui::Panel::left("rail")
            .exact_size(RAIL_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(PANEL).inner_margin(Margin {
                left: 10,
                right: 10,
                top: 22,
                bottom: 18,
            }))
            .show(ui, |ui| self.rail_ui(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .inner_margin(Margin::symmetric(30, 26)),
            )
            .show(ui, |ui| {
                ui.painter().vline(
                    ui.max_rect().left() - 30.0,
                    ui.max_rect().y_range().expand(26.0),
                    Stroke::new(1.0, BORDER),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width().min(1032.0);
                        ui.vertical_centered(|ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| match self.page {
                                    Page::Consoles => self.consoles_page(ui, &ctx, &rows),
                                    Page::Settings => self.settings_page(ui),
                                    Page::Log => log_page(ui, self.config.settings.streamer_mode),
                                },
                            );
                        });
                    });
            });

        if self.account_guide_open {
            self.account_guide_open = account_id_guide(&ctx);
        } else {
            self.register_dialog(&ctx);
        }
    }

    fn rail_ui(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            self.logo.show(ui, 40.0).on_hover_text("Bakery PS Stream");
            ui.add_space(15.0);
            theme::divider(ui);
            ui.add_space(10.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            for (page, icon, tooltip) in [
                (Page::Consoles, Icon::Home, "Consoles"),
                (Page::Settings, Icon::Gear, "Settings"),
                (Page::Log, Icon::List, "Logs"),
            ] {
                if theme::nav_item(ui, icon, tooltip, self.page == page).clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn consoles_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rows: &[HostRow]) {
        ui.horizontal_centered(|ui| {
            theme::page_heading(ui, "Consoles");
            if !rows.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, Icon::Plus, "Add console", false).clicked() {
                        self.open_manual_registration();
                    }
                });
            }
        });
        ui.add_space(18.0);

        let streamer_mode = self.config.settings.streamer_mode;
        if let Some((message, is_error)) = &self.banner {
            let summary = if *is_error {
                "Console action failed."
            } else {
                "Console action completed."
            };
            theme::notice(ui, screen_text(streamer_mode, message, summary), *is_error);
            ui.add_space(12.0);
        }

        let selected = self
            .selected
            .as_ref()
            .and_then(|key| rows.iter().position(|row| &row.key() == key))
            .or_else(|| {
                rows.iter()
                    .position(|row| row.awake && row.link().is_some())
            })
            .or_else(|| rows.iter().position(|row| row.link().is_some()))
            .or_else(|| (!rows.is_empty()).then_some(0));

        let Some(selected) = selected else {
            self.empty_console_panel(ui);
            return;
        };

        let row = &rows[selected];
        if let Some(action) = self.console_details(ui, row, selected) {
            self.do_action(ctx, row, action);
        }
        ui.add_space(24.0);
        if rows.len() == 1 {
            let (label, color) = self.discovery_status();
            theme::status_badge(ui, label, color);
            return;
        }
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Your consoles")
                    .size(15.0)
                    .family(theme::semibold()),
            );
            ui.label(
                RichText::new(rows.len().to_string())
                    .size(13.0)
                    .color(TEXT_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, color) = self.discovery_status();
                theme::status_badge(ui, label, color);
            });
        });
        ui.add_space(8.0);
        let tile_width = (ui.available_width() - 24.0) / 3.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(12.0);
            for (index, row) in rows.iter().enumerate() {
                let item = ui.allocate_ui_with_layout(
                    Vec2::new(tile_width, 68.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| console_tile(ui, row, index, index == selected, streamer_mode),
                );
                if item.inner.clicked() {
                    self.selected = Some(row.key());
                }
            }
        });
    }

    fn open_manual_registration(&mut self) {
        self.register = Some(new_register(String::new(), true, true, false));
    }

    fn discovery_status(&self) -> (&'static str, Option<Color32>) {
        if self.discovery.is_some() {
            ("Local discovery active", Some(LIVE))
        } else {
            ("Discovery off", None)
        }
    }

    fn console_details(
        &mut self,
        ui: &mut egui::Ui,
        row: &HostRow,
        index: usize,
    ) -> Option<HostAction> {
        let mut action = None;
        let mut open_settings = false;
        let s = &self.config.settings;
        let streamer_mode = s.streamer_mode;
        let linked = row.link().is_some();

        // ---- Stage: the console, lit by its own status.
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 286.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme::RADIUS_CARD),
            PANEL,
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        let light = painter.with_clip_rect(rect.shrink(1.5));
        let (status, tint) = row.status();
        let tint = tint.unwrap_or(Color32::from_rgb(0x7a, 0x80, 0x8c));
        theme::paint_glow(
            &light,
            rect.left_top() + Vec2::new(width * 0.22, -70.0),
            Vec2::new(width * 0.55, 230.0),
            theme::alpha(ACCENT, 13),
        );
        let art_x = rect.right() - (width * 0.2).clamp(150.0, 205.0);
        let art = egui::pos2(art_x, rect.center().y);
        theme::paint_glow(
            &light,
            art,
            Vec2::splat(185.0),
            theme::alpha(tint, if row.awake { 15 } else { 7 }),
        );
        for (r, a) in [(68.0, 0x10), (108.0, 0x0a), (150.0, 0x06)] {
            light.circle_stroke(art, r, Stroke::new(1.0, white(a)));
        }
        theme::paint_icon(
            &light,
            Icon::Gamepad,
            art,
            86.0,
            if row.awake { white(0xf4) } else { white(0x8a) },
        );

        let left = rect.left() + 34.0;
        let text_right = (art_x - 190.0).max(left + 280.0);
        let mut top = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(left, rect.top() + 32.0),
                    egui::pos2(text_right, rect.bottom() - 100.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        top.spacing_mut().item_spacing.y = 6.0;
        top.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 6.0, theme::alpha(tint, 20));
            ui.painter().circle_filled(dot.center(), 3.0, tint);
            ui.label(
                RichText::new(status)
                    .size(12.0)
                    .family(theme::semibold())
                    .color(tint),
            );
            let address = screen_text(streamer_mode, &row.addr, "Address hidden");
            ui.label(
                RichText::new(format!("·  {}  ·  {address}", row.model()))
                    .size(12.0)
                    .color(TEXT_DIM),
            );
        });
        top.add(
            egui::Label::new(
                RichText::new(row.display_name(index, streamer_mode))
                    .size(36.0)
                    .family(theme::semibold())
                    .extra_letter_spacing(-1.1),
            )
            .truncate(),
        );
        if !streamer_mode && !row.running_app.is_empty() {
            top.horizontal(|ui| {
                ui.label(RichText::new("Now playing").size(12.5).color(TEXT_DIM));
                ui.add(
                    egui::Label::new(
                        RichText::new(&row.running_app)
                            .size(12.5)
                            .family(theme::semibold()),
                    )
                    .truncate(),
                );
            });
        } else if !linked {
            top.label(
                RichText::new("Link it once to Play and Watch from this PC.")
                    .size(12.5)
                    .color(TEXT_DIM),
            );
        }

        let mut bottom = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(left, rect.bottom() - 96.0),
                    egui::pos2(rect.right() - 28.0, rect.bottom() - 26.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        bottom.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if !linked {
                if theme::big_button(ui, Icon::Plus, "Link console").clicked() {
                    action = Some(HostAction::Register);
                }
            } else if row.standby && row.seen {
                if theme::big_button(ui, Icon::Power, "Wake console").clicked() {
                    action = Some(HostAction::Wake);
                }
                ui.label(
                    RichText::new("In rest mode. Waking takes about 20 seconds.")
                        .size(12.0)
                        .color(TEXT_DIM),
                );
            } else {
                let reachable = row.awake || !row.seen;
                ui.add_enabled_ui(reachable && row.saved.is_some(), |ui| {
                    if theme::big_button(ui, Icon::Play, "Play")
                        .on_hover_text("Stream with a controller or keyboard on this PC")
                        .clicked()
                    {
                        action = Some(HostAction::Play);
                    }
                });
                ui.add_enabled_ui(reachable, |ui| {
                    if theme::big_outline_button(ui, Icon::Eye, "Watch")
                        .on_hover_text("Stream without sending any input from this PC")
                        .clicked()
                    {
                        action = Some(HostAction::Watch);
                    }
                });
            }
        });
        bottom.add_space(4.0);
        bottom.horizontal(|ui| {
            let codec = if s.codec == 0 { "H.264" } else { "H.265" };
            let quality = format!("{} · {} fps · {codec}", res_name(s.resolution), s.fps);
            ui.label(RichText::new(quality).size(11.5).color(TEXT_DIM));
            open_settings |= ui
                .link(RichText::new("Change").size(11.5))
                .on_hover_text("Open video settings")
                .clicked();
            if linked {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .link(RichText::new("Forget console").size(11.5).color(TEXT_DIM))
                        .on_hover_text("Remove every saved link for this console")
                        .clicked()
                    {
                        action = Some(HostAction::Forget);
                    }
                });
            }
        });

        // ---- Optional second account, explained by what it gets you.
        if row.saved.is_some() {
            ui.add_space(12.0);
            let has_second = row.watch_saved.is_some();
            theme::panel()
                .inner_margin(Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        let (badge, _) =
                            ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                        ui.painter().circle_filled(badge.center(), 18.0, POPOVER);
                        theme::paint_icon(
                            ui.painter(),
                            if has_second { Icon::Eye } else { Icon::User },
                            badge.center(),
                            16.0,
                            if has_second { LIVE } else { TEXT },
                        );
                        let text_width = (ui.available_width() - 190.0).max(220.0);
                        ui.vertical(|ui| {
                            ui.set_width(text_width);
                            ui.spacing_mut().item_spacing.y = 3.0;
                            let (title, body) = if has_second {
                                (
                                    "Watch uses your second account",
                                    "Your main account and controller stay in charge on the console while you watch here.",
                                )
                            } else {
                                (
                                    "Watch without taking over the console",
                                    "Watch already works with this link. Add a second PSN account and Watch uses it instead, so your main account keeps playing on the console undisturbed.",
                                )
                            };
                            ui.label(RichText::new(title).size(13.5).family(theme::semibold()));
                            ui.label(RichText::new(body).size(12.0).color(TEXT_DIM));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if has_second {
                                if ui
                                    .link(RichText::new("Remove").size(12.0).color(TEXT_DIM))
                                    .clicked()
                                {
                                    action = Some(HostAction::ForgetWatchAccount);
                                }
                            } else if theme::icon_button(ui, Icon::Plus, "Add second account", false)
                                .clicked()
                            {
                                action = Some(HostAction::RegisterWatchAccount);
                            }
                        });
                    });
                });
        }

        if open_settings {
            self.settings_section = SettingsSection::Video;
            self.page = Page::Settings;
        }
        action
    }

    fn empty_console_panel(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 330.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme::RADIUS_CARD),
            PANEL,
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        let light = painter.with_clip_rect(rect.shrink(1.5));
        let art = rect.center() - Vec2::new(0.0, 62.0);
        theme::paint_glow(
            &light,
            art,
            Vec2::new(260.0, 170.0),
            theme::alpha(ACCENT, 14),
        );
        // Slow ripple while searching.
        let t = ui.input(|i| i.time) as f32;
        for (k, a) in [(0.0f32, 0x14u8), (0.5, 0x0a)] {
            let phase = (t * 0.45 + k) % 1.0;
            light.circle_stroke(
                art,
                40.0 + phase * 90.0,
                Stroke::new(1.0, white(((1.0 - phase) * a as f32) as u8)),
            );
        }
        theme::paint_icon(&light, Icon::Gamepad, art, 60.0, white(0xe8));
        ui.ctx().request_repaint_after(Duration::from_millis(33));

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_center_size(
                    rect.center() + Vec2::new(0.0, 74.0),
                    Vec2::new(rect.width().min(520.0), 140.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        child.label(
            RichText::new("Looking for your PlayStation")
                .size(22.0)
                .family(theme::semibold())
                .extra_letter_spacing(-0.5),
        );
        child.label(
            RichText::new("Turn it on, or put it in rest mode, on the same network as this PC.")
                .size(12.5)
                .color(TEXT_DIM),
        );
        child.add_space(10.0);
        if theme::icon_button(&mut child, Icon::Plus, "Add by IP address", false).clicked() {
            self.open_manual_registration();
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        theme::page_heading(ui, "Settings");
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            for (section, label) in [
                (SettingsSection::Account, "Account & privacy"),
                (SettingsSection::Video, "Video"),
                (SettingsSection::Playback, "Playback"),
            ] {
                let selected = self.settings_section == section;
                let tab = egui::Button::selectable(
                    selected,
                    RichText::new(label)
                        .family(theme::semibold())
                        .size(12.5)
                        .color(if selected { TEXT } else { TEXT_DIM }),
                )
                .fill(if selected {
                    POPOVER
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(if selected {
                    Stroke::new(1.0, BORDER)
                } else {
                    Stroke::NONE
                })
                .min_size(Vec2::new(72.0, 33.0));
                if ui.add(tab).clicked() {
                    self.settings_section = section;
                }
            }
        });
        ui.add_space(16.0);
        let mut changed = false;
        let s = &mut self.config.settings;

        if let Some((message, true)) = &self.banner {
            theme::notice(
                ui,
                screen_text(
                    s.streamer_mode,
                    message,
                    "Settings or console action failed.",
                ),
                true,
            );
            ui.add_space(14.0);
        }
        match self.settings_section {
            SettingsSection::Account => settings_panel(ui, |ui| {
                let (id_text, id_color) = match s.account_id_bytes() {
                    Ok(_) => ("Valid account ID".to_owned(), LIVE),
                    Err(err) if !s.psn_account_id.is_empty() => (err, theme::DESTRUCTIVE_TEXT),
                    Err(_) => ("Not set".to_owned(), TEXT_DIM),
                };
                settings_row(
                    ui,
                    Icon::User,
                    "PSN account ID",
                    "Your main account’s base64 ID. One link with it covers Play and Watch.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut s.psn_account_id)
                                    .password(true)
                                    .hint_text("Base64 account ID")
                                    .desired_width(192.0)
                                    .margin(Margin::symmetric(10, 8)),
                            )
                            .changed();
                    },
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(id_text).size(11.0).color(id_color));
                            if ui.link("How to find it").clicked() {
                                self.account_guide_open = true;
                            }
                        });
                    },
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::Monitor,
                    "Streamer mode",
                    "Hide console details, pairing fields and logs while sharing your screen.",
                    |ui| {
                        changed |= theme::switch(ui, &mut s.streamer_mode).changed();
                    },
                    |_| {},
                );
            }),
            SettingsSection::Video => settings_panel(ui, |ui| {
                settings_row(
                    ui,
                    Icon::Monitor,
                    "Resolution",
                    "1080p requires a PS5 or PS4 Pro.",
                    |ui| {
                        egui::ComboBox::new("res", "")
                            .width(136.0)
                            .selected_text(res_name(s.resolution))
                            .show_ui(ui, |ui| {
                                for resolution in [4, 3, 2, 1] {
                                    changed |= ui
                                        .selectable_value(
                                            &mut s.resolution,
                                            resolution,
                                            res_name(resolution),
                                        )
                                        .changed();
                                }
                            });
                    },
                    |_| {},
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::Play,
                    "Frame rate",
                    "Use 30 fps if the stream stutters on your network.",
                    |ui| {
                        segmented(
                            ui,
                            &mut s.fps,
                            &[(30, "30 fps"), (60, "60 fps")],
                            &mut changed,
                        );
                    },
                    |_| {},
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::Play,
                    "Codec",
                    "H.265 uses bandwidth more efficiently on PS5.",
                    |ui| {
                        segmented(
                            ui,
                            &mut s.codec,
                            &[(0, "H.264"), (1, "H.265")],
                            &mut changed,
                        );
                    },
                    |_| {},
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::List,
                    "Bitrate",
                    "Set to 0 to use the console’s default for the resolution.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.bitrate_kbps)
                                    .range(0..=100_000)
                                    .speed(100)
                                    .suffix(" kbps"),
                            )
                            .changed();
                    },
                    |_| {},
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::Gear,
                    "Hardware decoding",
                    "Use D3D11VA when the GPU supports it.",
                    |ui| {
                        changed |= theme::switch(ui, &mut s.hw_decode).changed();
                    },
                    |_| {},
                );
            }),
            SettingsSection::Playback => settings_panel(ui, |ui| {
                settings_row(
                    ui,
                    Icon::Gamepad,
                    "Stream volume",
                    "Adjust audio playback on this PC.",
                    |ui| {
                        changed |= volume_slider(ui, &mut s.volume_percent).changed();
                    },
                    |_| {},
                );
                theme::divider(ui);
                let keyboard_on = s.keyboard_controls;
                settings_row(
                    ui,
                    Icon::Gamepad,
                    "Keyboard controls",
                    "Enable keyboard input during Play sessions.",
                    |ui| {
                        changed |= theme::switch(ui, &mut s.keyboard_controls).changed();
                    },
                    |ui| {
                        if keyboard_on {
                            ui.label(
                                RichText::new(input::KEYBOARD_HELP)
                                    .size(11.0)
                                    .color(TEXT_DIM),
                            );
                        }
                    },
                );
                theme::divider(ui);
                settings_row(
                    ui,
                    Icon::Monitor,
                    "Fullscreen on connect",
                    "Use F11 to toggle fullscreen and Esc to return to windowed mode.",
                    |ui| {
                        changed |= theme::switch(ui, &mut s.fullscreen_on_connect).changed();
                    },
                    |_| {},
                );
            }),
        }
        ui.label(
            RichText::new(format!(
                "Bakery PS Stream {} · Powered by chiaki-ng",
                env!("CARGO_PKG_VERSION")
            ))
            .size(11.5)
            .color(TEXT_DIM),
        );
        if changed {
            self.config.save();
        }
    }

    fn register_dialog(&mut self, ctx: &egui::Context) {
        let streamer_mode = self.config.settings.streamer_mode;
        let Some(dlg) = self.register.as_mut() else {
            return;
        };

        if let Some((result, account)) = dlg
            .running
            .as_ref()
            .and_then(|(r, account)| r.take_result().map(|result| (result, *account)))
        {
            dlg.running = None;
            let result = result.and_then(|host| {
                let saved = SavedHost::from_registered(&host, &dlg.addr, account);
                if dlg
                    .expected_mac
                    .as_ref()
                    .is_some_and(|mac| *mac != saved.mac)
                {
                    return Err(
                        "A different console answered. Check the console address and try again."
                            .into(),
                    );
                }
                if dlg.watch_only {
                    let play_host = self.config.hosts.iter().find(|h| h.mac == saved.mac);
                    self.config.validate_watch_account(account, play_host)?;
                }
                Ok(saved)
            });
            match result {
                Ok(saved) => {
                    self.banner = Some((
                        if dlg.watch_only {
                            format!(
                                "Second account added. Watch on {} now uses it.",
                                saved.nickname
                            )
                        } else {
                            format!("Linked {}. You can Play or Watch now.", saved.nickname)
                        },
                        false,
                    ));
                    self.selected = Some(saved.mac.clone());
                    if !dlg.watch_only && self.config.settings.account_id_bytes().is_err() {
                        self.config.settings.psn_account_id = dlg.account_input.trim().to_owned();
                    }
                    self.config.upsert_host(saved, dlg.watch_only);
                    self.config.save();
                    self.page = Page::Consoles;
                    self.register = None;
                    return;
                }
                Err(e) => dlg.error = Some(e),
            }
        }

        let saved_play_id = self.config.settings.account_id_bytes().ok();
        match registration_form(ctx, dlg, saved_play_id, streamer_mode) {
            RegistrationAction::Guide => self.account_guide_open = true,
            RegistrationAction::Cancel => self.register = None,
            RegistrationAction::Link => {
                dlg.error = None;
                dlg.addr = dlg.addr.trim().to_owned();
                let result = registration_account(dlg, saved_play_id).and_then(|account| {
                    if dlg.watch_only {
                        let play_host = self
                            .config
                            .hosts
                            .iter()
                            .find(|host| host.last_addr == dlg.addr);
                        self.config.validate_watch_account(account, play_host)?;
                    }
                    let pin = dlg
                        .pin
                        .parse::<u32>()
                        .map_err(|_| "Enter the 8-digit pairing code.".to_owned())?;
                    Registration::start(ctx.clone(), &dlg.addr, dlg.ps5, &account, pin)
                        .map(|registration| (registration, account))
                        .map_err(|err| format!("{err:#}"))
                });
                match result {
                    Ok(running) => dlg.running = Some(running),
                    Err(err) => dlg.error = Some(err),
                }
            }
            RegistrationAction::None => {}
        }
    }

    // ---------------------------------------------------------- stream screen

    fn stream_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut leave = false;
        let mut toggle_fullscreen = false;
        let keyboard = self.config.settings.keyboard_controls;
        let streamer_mode = self.config.settings.streamer_mode;
        let mut volume_changed = false;

        let Some(st) = self.streaming.as_mut() else {
            return;
        };
        let status = st.session.status();

        // Newest frame -> GPU.
        if let Some(frame) = st.session.pull_frame()
            && !self.video.upload(&frame)
            && !st.unsupported_format_logged
        {
            st.unsupported_format_logged = true;
            remote::log_line(format!(
                "[E] Unsupported video pixel format {}",
                frame.format()
            ));
        }

        if ctx.input(|i| i.key_pressed(Key::F1)) {
            st.menu_open = !st.menu_open;
        }
        if ctx.input(|i| i.key_pressed(Key::F11) || (self.fullscreen && i.key_pressed(Key::Escape)))
        {
            toggle_fullscreen = true;
        }

        if self.fullscreen {
            let screen = ctx.content_rect();
            let show_controls = st.menu_open
                || ctx.input(|i| {
                    i.pointer
                        .hover_pos()
                        .is_some_and(|pos| pos.y < screen.top() + STREAM_TOOLBAR_HEIGHT)
                });
            if show_controls {
                egui::Area::new(egui::Id::unique("fullscreen-stream-controls"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(screen.left_top() + egui::vec2(10.0, 8.0))
                    .show(&ctx, |ui| {
                        egui::Frame::NONE
                            .fill(PANEL)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(8, 5))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    stream_controls(
                                        ui,
                                        st,
                                        true,
                                        streamer_mode,
                                        &status,
                                        &mut toggle_fullscreen,
                                        &mut leave,
                                    )
                                });
                            });
                    });
            }
        } else {
            egui::Panel::top("stream-toolbar")
                .exact_size(STREAM_TOOLBAR_HEIGHT)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(PANEL))
                .show(ui, |ui| {
                    let toolbar = ui.max_rect();
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(toolbar.shrink2(egui::vec2(12.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            stream_controls(
                                ui,
                                st,
                                false,
                                streamer_mode,
                                &status,
                                &mut toggle_fullscreen,
                                &mut leave,
                            )
                        },
                    );
                });
        }

        // Controller input, unless watching only or a text box needs the keyboard.
        let watch_only = st.watch_only;
        if !watch_only {
            let typing = matches!(status, SessionStatus::NeedsLoginPin(_)) || st.menu_open;
            let pad = self.input.poll(&ctx, keyboard && !typing);
            if status == SessionStatus::Connected {
                st.session.set_controller(pad);
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                self.video.paint(ui, rect);

                match &status {
                    SessionStatus::Connecting if self.video.frame_size().is_none() => {
                        overlay(ui, rect, |ui| {
                            theme::eyebrow(ui, "Connecting");
                            ui.label(
                                RichText::new(screen_text(streamer_mode, &st.title, "PlayStation"))
                                    .size(20.0)
                                    .family(theme::semibold()),
                            );
                            ui.add(egui::Spinner::new().size(18.0).color(TEXT_DIM));
                            if ui.add(theme::outline_button("Cancel")).clicked() {
                                leave = true;
                            }
                        });
                    }
                    SessionStatus::NeedsLoginPin(wrong) => {
                        let wrong = *wrong;
                        overlay(ui, rect, |ui| {
                            theme::eyebrow(ui, "Login passcode");
                            ui.label(
                                RichText::new("Enter passcode")
                                    .size(20.0)
                                    .family(theme::semibold()),
                            );
                            ui.label(
                                RichText::new("This console user has a passcode.")
                                    .size(11.0)
                                    .color(TEXT_DIM),
                            );
                            if wrong {
                                ui.label(
                                    RichText::new("That passcode was wrong, try again.")
                                        .size(11.0)
                                        .color(theme::DESTRUCTIVE_TEXT),
                                );
                            }
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut st.pin_input)
                                    .password(true)
                                    .char_limit(4)
                                    .desired_width(120.0)
                                    .align(egui::Align2::CENTER_CENTER)
                                    .font(egui::FontId::new(20.0, theme::semibold()))
                                    .margin(Margin::symmetric(10, 8)),
                            );
                            resp.request_focus();
                            let submit = ui.add(theme::primary_button("Continue")).clicked()
                                || (resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                            if submit && !st.pin_input.is_empty() {
                                st.session.set_login_pin(&st.pin_input);
                                st.pin_input.clear();
                            }
                        });
                    }
                    SessionStatus::Ended(reason) => {
                        let reason = reason.clone();
                        overlay(ui, rect, |ui| {
                            theme::eyebrow(ui, "Session ended");
                            ui.label(
                                RichText::new("Disconnected")
                                    .size(20.0)
                                    .family(theme::semibold()),
                            );
                            if !streamer_mode {
                                ui.label(RichText::new(reason).size(11.0).color(TEXT_DIM));
                            }
                            if ui.add(theme::primary_button("Back to consoles")).clicked() {
                                leave = true;
                            }
                        });
                    }
                    _ => {}
                }

                if st.menu_open {
                    overlay(ui, rect, |ui| {
                        ui.label(
                            RichText::new("Stream controls")
                                .size(21.0)
                                .family(theme::semibold()),
                        );
                        let name = if streamer_mode {
                            "PlayStation".to_owned()
                        } else {
                            st.session.nickname().unwrap_or_else(|| st.title.clone())
                        };
                        ui.label(RichText::new(name).size(12.0).color(TEXT_DIM));
                        let pad = if st.using_second_account {
                            "Use your main account with the controller on the console.".to_owned()
                        } else if watch_only {
                            "PC controls are off.".to_owned()
                        } else if streamer_mode {
                            "PC controls".to_owned()
                        } else {
                            self.input
                                .active_gamepad_name()
                                .unwrap_or_else(|| "No controller detected".into())
                        };
                        ui.horizontal(|ui| {
                            theme::icon(ui, Icon::Gamepad, 15.0, TEXT_DIM);
                            ui.label(RichText::new(pad).size(11.0).color(TEXT_DIM));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Volume");
                            if volume_slider(ui, &mut self.config.settings.volume_percent).changed()
                            {
                                st.session.set_volume(self.config.settings.volume_percent);
                                volume_changed = true;
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if !watch_only && ui.add(theme::outline_button("PS Home")).clicked() {
                                st.session.go_home();
                            }
                            if ui
                                .add(theme::outline_button(if self.fullscreen {
                                    "Windowed"
                                } else {
                                    "Fullscreen"
                                }))
                                .clicked()
                            {
                                toggle_fullscreen = true;
                            }
                            if !watch_only && ui.add(theme::outline_button("Rest mode")).clicked() {
                                st.session.goto_bed();
                            }
                            if ui.add(theme::danger_button("Disconnect")).clicked() {
                                leave = true;
                            }
                        });
                        if keyboard && !watch_only {
                            ui.label(
                                RichText::new(input::KEYBOARD_HELP)
                                    .size(10.5)
                                    .color(TEXT_DIM),
                            );
                        }
                        ui.label(
                            RichText::new("F1 closes this menu")
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                    });
                } else if self.fullscreen && status == SessionStatus::Connected {
                    ui.painter().text(
                        rect.right_bottom() - egui::vec2(10.0, 10.0),
                        egui::Align2::RIGHT_BOTTOM,
                        if watch_only {
                            "WATCH ONLY  ·  ESC  WINDOWED  ·  F1  MENU"
                        } else {
                            "ESC  WINDOWED  ·  F1  MENU"
                        },
                        egui::FontId::new(9.5, theme::extrabold()),
                        white(0x30),
                    );
                }
            });

        if volume_changed {
            self.config.save();
        }
        if leave {
            self.streaming = None; // stops and joins the session
            self.video.clear();
            if self.fullscreen {
                self.set_fullscreen(&ctx, false);
            }
            ctx.request_repaint();
            return;
        }
        if toggle_fullscreen {
            let on = !self.fullscreen;
            self.set_fullscreen(&ctx, on);
        }
        if !watch_only {
            // Keep polling the gamepad even when no new video frame arrives.
            ctx.request_repaint_after(Duration::from_millis(4));
        } else {
            // Watching: new frames trigger repaints; this only refreshes status.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

fn screen_text<'a>(streamer_mode: bool, text: &'a str, hidden: &'a str) -> &'a str {
    if streamer_mode { hidden } else { text }
}

fn volume_slider(ui: &mut egui::Ui, percent: &mut u8) -> egui::Response {
    ui.add(egui::Slider::new(percent, 0..=100).suffix("%"))
}

fn stream_controls(
    ui: &mut egui::Ui,
    stream: &mut Streaming,
    fullscreen: bool,
    streamer_mode: bool,
    status: &SessionStatus,
    toggle_fullscreen: &mut bool,
    leave: &mut bool,
) {
    let (state, color) = match status {
        SessionStatus::Connected => (
            if stream.watch_only {
                "Watch only"
            } else {
                "Playing"
            },
            LIVE,
        ),
        SessionStatus::Connecting => ("Connecting", TEXT_DIM),
        SessionStatus::NeedsLoginPin(_) => ("Passcode required", STANDBY),
        SessionStatus::Ended(_) => ("Disconnected", TEXT_DIM),
    };
    theme::icon(ui, Icon::Gamepad, 19.0, TEXT_DIM);
    ui.vertical(|ui| {
        ui.set_max_width(270.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let name = screen_text(streamer_mode, &stream.title, "PlayStation");
        ui.add(
            egui::Label::new(RichText::new(name).size(13.0).family(theme::semibold())).truncate(),
        );
        ui.label(RichText::new(state).size(11.0).color(color));
    });
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(theme::danger_button("Disconnect")).clicked() {
            *leave = true;
        }
        let display = if fullscreen { "Windowed" } else { "Fullscreen" };
        if ui
            .add(theme::outline_button(display))
            .on_hover_text("F11")
            .clicked()
        {
            *toggle_fullscreen = true;
        }
        let menu = if stream.menu_open {
            "Close menu"
        } else {
            "Menu"
        };
        if ui
            .add(theme::outline_button(menu))
            .on_hover_text("F1")
            .clicked()
        {
            stream.menu_open = !stream.menu_open;
        }
    });
}

fn new_register(addr: String, ps5: bool, manual: bool, watch_only: bool) -> RegisterDialog {
    RegisterDialog {
        addr,
        ps5,
        manual,
        watch_only,
        expected_mac: None,
        account_input: String::new(),
        focus_first: true,
        pin: String::new(),
        running: None,
        error: None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegistrationAction {
    None,
    Guide,
    Cancel,
    Link,
}

fn registration_account(
    dlg: &RegisterDialog,
    saved_play_id: Option<[u8; 8]>,
) -> Result<[u8; 8], String> {
    if let Some(id) = saved_play_id.filter(|_| !dlg.watch_only) {
        return Ok(id);
    }
    decode_account_id(&dlg.account_input)
}

fn console_address_ready(value: &str) -> bool {
    let address = value.trim();
    !address.is_empty()
        && !address
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '/' | '\\'))
}

fn setup_modal(name: &'static str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(name))
        .backdrop_color(Color32::from_black_alpha(155))
        .frame(theme::panel().fill(POPOVER).inner_margin(Margin::same(23)))
}

fn setup_help(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(TEXT_DIM));
}

fn account_id_guide(ctx: &egui::Context) -> bool {
    let modal = setup_modal("account-id-guide").show(ctx, |ui| {
        ui.set_width(440.0);
        theme::page_heading(ui, "Find your PSN account ID");
        setup_help(ui, "Bakery needs your Base64 account ID, which is separate from the username people see on PlayStation Network.");
        ui.add_space(12.0);
        for step in [
            "1. Open Pyrosoft’s finder and select Sign in with PlayStation to sign in on Sony’s page.",
            "2. Copy the full address from the page Sony redirects you to. Paste it into Pyrosoft’s Redirect URL field and select Get my Account ID.",
            "3. Copy the Base64 account ID into Bakery’s PSN account ID field.",
        ] {
            ui.label(step);
            ui.add_space(4.0);
        }
        setup_help(ui, "You can also use PSN Login in chiaki-ng’s registration dialog and copy its PSN Account-ID field.");
        ui.hyperlink_to("chiaki-ng setup instructions", PSN_ACCOUNT_GUIDE);
        ui.add_space(14.0);
        ui.horizontal_centered(|ui| {
            ui.hyperlink_to("Open Pyrosoft’s account ID finder", PSN_ID_FINDER);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(theme::primary_button("Back")).clicked()
            }).inner
        }).inner
    });
    !modal.inner && !modal.should_close()
}

fn registration_form(
    ctx: &egui::Context,
    dlg: &mut RegisterDialog,
    saved_play_id: Option<[u8; 8]>,
    streamer_mode: bool,
) -> RegistrationAction {
    let mut action = RegistrationAction::None;
    let running = dlg.running.is_some();
    let uses_saved_account = !dlg.watch_only && saved_play_id.is_some();
    let focus_first = std::mem::take(&mut dlg.focus_first);
    let (title, intro) = if dlg.watch_only {
        (
            "Add a second account",
            "Optional. Watch streams through this account instead of your main one, so your main account and controller keep playing on the console undisturbed.",
        )
    } else if dlg.manual {
        (
            "Add console",
            "One link covers both Play and Watch. Keep this PC and the console on the same network.",
        )
    } else {
        (
            "Link console",
            "One link covers both Play and Watch. You only do this once per console.",
        )
    };
    let modal = setup_modal("register-console").show(ctx, |ui| {
        ui.set_width(452.0);
        ui.label(
            RichText::new(title)
                .size(23.0)
                .family(theme::semibold())
                .extra_letter_spacing(-0.5),
        );
        ui.add_space(2.0);
        setup_help(ui, intro);
        ui.add_space(16.0);
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 190.0).max(220.0))
            .show(ui, |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    // 1. Console
                    step(ui, 1, console_address_ready(&dlg.addr), |ui| {
                        field_label(ui, "Console");
                        if dlg.manual {
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut dlg.addr)
                                        .password(streamer_mode)
                                        .hint_text(if streamer_mode {
                                            "Address hidden as you type"
                                        } else {
                                            "Console IP address"
                                        })
                                        .desired_width((ui.available_width() - 156.0).max(135.0))
                                        .margin(Margin::symmetric(10, 8)),
                                );
                                if focus_first {
                                    response.request_focus();
                                }
                                let mut ps4 = !dlg.ps5;
                                segmented_bool(ui, &mut ps4, "PS4", "PS5", &mut false);
                                dlg.ps5 = !ps4;
                            });
                            setup_help(ui, if dlg.ps5 {
                                "Settings > System > System Software > Console Information shows the IP."
                            } else {
                                "Settings > Network > View Connection Status shows the IP."
                            });
                            if !dlg.addr.trim().is_empty() && !console_address_ready(&dlg.addr) {
                                ui.label(
                                    RichText::new("Enter the address without spaces or a URL.")
                                        .size(12.0)
                                        .color(theme::DESTRUCTIVE_TEXT),
                                );
                            }
                        } else {
                            let model = if dlg.ps5 { "PlayStation 5" } else { "PlayStation 4" };
                            let address = screen_text(streamer_mode, &dlg.addr, "Address hidden");
                            setup_help(ui, &format!("{model} · {address}"));
                        }
                    });

                    // 2. Account
                    let account_ok = registration_account(dlg, saved_play_id).is_ok();
                    step(ui, 2, account_ok, |ui| {
                        let label =
                            if dlg.watch_only { "Second PSN account ID" } else { "PSN account ID" };
                        if uses_saved_account {
                            field_label(ui, label);
                        } else {
                            ui.horizontal(|ui| {
                                field_label(ui, label);
                                if ui.link("How to find it").clicked() {
                                    action = RegistrationAction::Guide;
                                }
                            });
                        }
                        if uses_saved_account {
                            setup_help(ui, "Using the main account saved in Settings.");
                        } else {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut dlg.account_input)
                                    .password(true)
                                    .hint_text("Base64 account ID")
                                    .desired_width(f32::INFINITY)
                                    .margin(Margin::symmetric(10, 8)),
                            );
                            if focus_first && !dlg.manual {
                                response.request_focus();
                            }
                            if !dlg.account_input.trim().is_empty()
                                && let Err(err) = registration_account(dlg, saved_play_id)
                            {
                                ui.label(
                                    RichText::new(err).size(12.0).color(theme::DESTRUCTIVE_TEXT),
                                );
                            }
                            if dlg.watch_only {
                                setup_help(ui, "Use a different account from your main one.");
                            }
                        }
                    });

                    // 3. Pairing code
                    let pin_ok = dlg.pin.len() == 8;
                    step(ui, 3, pin_ok, |ui| {
                        field_label(ui, "Pairing code");
                        let path = if dlg.ps5 {
                            "Settings > System > Remote Play > Link Device"
                        } else {
                            "Settings > Remote Play Connection Settings > Add Device"
                        };
                        setup_help(
                            ui,
                            &if dlg.watch_only {
                                format!("Sign in to the console as the second account, then open {path}.")
                            } else {
                                format!("On the console, open {path}.")
                            },
                        );
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut dlg.pin)
                                .password(streamer_mode)
                                .hint_text("8-digit code")
                                .font(egui::FontId::new(17.0, theme::semibold()))
                                .desired_width(170.0)
                                .margin(Margin::symmetric(12, 8)),
                        );
                        if focus_first && !dlg.manual && uses_saved_account {
                            response.request_focus();
                        }
                        dlg.pin.retain(|ch| ch.is_ascii_digit());
                        dlg.pin.truncate(8);
                        if action == RegistrationAction::None
                            && can_link_console(dlg, saved_play_id)
                            && response.lost_focus()
                            && ui.input(|i| i.key_pressed(Key::Enter))
                        {
                            action = RegistrationAction::Link;
                        }
                    });
                });
                if let Some(err) = &dlg.error {
                    ui.add_space(8.0);
                    let summary =
                        "Could not link. Check the console address, account ID and pairing code.";
                    theme::notice(ui, screen_text(streamer_mode, err, summary), true);
                }
            });
        ui.add_space(18.0);
        ui.horizontal_centered(|ui| {
            if running {
                ui.add(egui::Spinner::new().size(15.0));
                setup_help(ui, "Waiting for the console…");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let submit = if dlg.watch_only { "Add account" } else { "Link console" };
                if !running
                    && ui
                        .add_enabled(
                            can_link_console(dlg, saved_play_id),
                            theme::primary_button(submit),
                        )
                        .clicked()
                {
                    action = RegistrationAction::Link;
                }
                if ui.add(theme::outline_button("Cancel")).clicked() {
                    action = RegistrationAction::Cancel;
                }
            });
        });
    });
    if modal.should_close() && !running && action == RegistrationAction::None {
        return RegistrationAction::Cancel;
    }
    action
}

/// One numbered step of the link dialog, with a guide line down to the next.
fn step(ui: &mut egui::Ui, n: u8, done: bool, add: impl FnOnce(&mut egui::Ui)) {
    let response = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        theme::step_number(ui, n, done);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui);
        });
    });
    if n < 3 {
        let r = response.response.rect;
        let x = r.left() + 11.0;
        ui.painter().vline(
            x,
            (r.top() + 26.0)..=(r.bottom() + 10.0),
            Stroke::new(1.0, white(0x18)),
        );
    }
    ui.add_space(14.0);
}

fn can_link_console(dlg: &RegisterDialog, saved_play_id: Option<[u8; 8]>) -> bool {
    console_address_ready(&dlg.addr)
        && dlg.pin.len() == 8
        && dlg.pin.bytes().all(|ch| ch.is_ascii_digit())
        && registration_account(dlg, saved_play_id).is_ok()
}

// ------------------------------------------------------------------ pieces

fn console_tile(
    ui: &mut egui::Ui,
    row: &HostRow,
    index: usize,
    selected: bool,
    streamer_mode: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 68.0), egui::Sense::click());
    let name = row.display_name(index, streamer_mode);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
    let hot = response.hovered() || response.has_focus();
    let (fill, border) = if selected {
        (POPOVER, white(0x30))
    } else if hot {
        (Color32::from_rgb(0x15, 0x17, 0x1c), white(0x22))
    } else {
        (PANEL, BORDER)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::RADIUS_CARD),
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
    let (status, color) = row.status();
    let tint = color.unwrap_or(Color32::from_rgb(0x7a, 0x80, 0x8c));
    if selected {
        theme::paint_glow(
            &painter,
            rect.left_center(),
            Vec2::new(140.0, 70.0),
            theme::alpha(ACCENT, 12),
        );
    }

    // Icon well, with the status light on its corner.
    let well = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 34.0, rect.center().y),
        Vec2::splat(40.0),
    );
    painter.rect_filled(well, CornerRadius::same(10), white(0x08));
    theme::paint_icon(
        &painter,
        Icon::Gamepad,
        well.center(),
        20.0,
        if row.awake { TEXT } else { white(0x90) },
    );
    let light = well.right_bottom() - Vec2::new(3.0, 3.0);
    painter.circle_filled(light, 5.5, fill);
    painter.circle_filled(light, 3.5, tint);

    let left = well.right() + 14.0;
    let mut label = egui::text::LayoutJob::simple_singleline(
        name,
        egui::FontId::new(13.5, theme::semibold()),
        TEXT,
    );
    label.wrap.max_width = rect.right() - left - 12.0;
    label.wrap.max_rows = 1;
    painter.galley(
        egui::pos2(left, rect.top() + 15.0),
        painter.layout_job(label),
        TEXT,
    );
    let detail = if row.link().is_some() {
        row.model()
    } else {
        "Not linked"
    };
    painter.text(
        egui::pos2(left, rect.top() + 44.0),
        egui::Align2::LEFT_CENTER,
        format!("{status} · {detail}"),
        egui::FontId::proportional(11.5),
        if color.is_some() { tint } else { TEXT_DIM },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Sample consoles for screenshots and UI work (debug builds, BAKERY_PS_DEMO=1).
#[cfg(debug_assertions)]
fn demo_rows() -> Vec<HostRow> {
    let saved = |name: &str, mac: &str, ps5: bool, addr: &str| SavedHost {
        nickname: name.into(),
        mac: mac.into(),
        ps5,
        last_addr: addr.into(),
        regist_key: String::new(),
        rp_key: String::new(),
        account_id: Some([1; 8]),
    };
    vec![
        HostRow {
            name: "Living Room PS5".into(),
            addr: "192.0.2.42".into(),
            ps5: true,
            awake: true,
            standby: false,
            seen: true,
            running_app: "Astro Bot".into(),
            saved: Some(saved("Living Room PS5", "0000000000a1", true, "192.0.2.42")),
            watch_saved: None,
        },
        HostRow {
            name: "Bedroom PS5".into(),
            addr: "192.0.2.57".into(),
            ps5: true,
            awake: false,
            standby: true,
            seen: true,
            running_app: String::new(),
            saved: Some(saved("Bedroom PS5", "0000000000a2", true, "192.0.2.57")),
            watch_saved: None,
        },
        HostRow {
            name: "Office PS4".into(),
            addr: "192.0.2.80".into(),
            ps5: false,
            awake: true,
            standby: false,
            seen: true,
            running_app: String::new(),
            saved: None,
            watch_saved: None,
        },
    ]
}

fn log_page(ui: &mut egui::Ui, streamer_mode: bool) {
    theme::page_heading(ui, "Logs");
    ui.add_space(18.0);
    if streamer_mode {
        theme::notice(ui, "Logs are hidden in Streamer mode.", false);
        return;
    }
    theme::panel().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let lines = remote::log_snapshot();
        egui::Frame::NONE
            .fill(Color32::from_rgb(0x0d, 0x0e, 0x12))
            .corner_radius(CornerRadius {
                nw: 0,
                ne: 0,
                sw: theme::RADIUS_CARD - 1,
                se: theme::RADIUS_CARD - 1,
            })
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(380.0);
                if lines.is_empty() {
                    ui.label(RichText::new("No output yet.").monospace().color(TEXT_DIM));
                }
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for line in lines {
                            let (tag, rest) = line.split_at(line.find(']').map_or(0, |i| i + 1));
                            let (kind, color) = match tag {
                                "[E]" => ("ERR", theme::DESTRUCTIVE_TEXT),
                                "[W]" => ("WARN", STANDBY),
                                _ => ("OUT", Color32::from_rgb(0xa4, 0xb1, 0xc4)),
                            };
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 18.0;
                                ui.add_sized(
                                    [36.0, 14.0],
                                    egui::Label::new(RichText::new(kind).monospace().color(color)),
                                );
                                ui.label(
                                    RichText::new(rest.trim())
                                        .monospace()
                                        .color(Color32::from_rgb(0xd8, 0xdd, 0xe5)),
                                );
                            });
                        }
                    });
            });
    });
}

/// `.settings-row`: icon box, title + description, control on the right.
fn settings_panel(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    theme::panel().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
    });
    ui.add_space(19.0);
}

fn settings_row(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    desc: &str,
    control: impl FnOnce(&mut egui::Ui),
    extra: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(8), POPOVER);
                theme::paint_icon(ui.painter(), icon, rect.center(), 15.0, TEXT_DIM);
                let content_width = (ui.available_width() - 240.0).max(170.0);
                ui.vertical(|ui| {
                    ui.set_width(content_width);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(title).size(13.0).family(theme::semibold()));
                    ui.label(RichText::new(desc).size(12.0).color(TEXT_DIM));
                    extra(ui);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
            });
        });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).family(theme::semibold()));
}

fn segmented(ui: &mut egui::Ui, value: &mut i32, options: &[(i32, &str)], changed: &mut bool) {
    egui::Frame::NONE
        .fill(BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS_BUTTON))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let options = if ui.layout().prefer_right_to_left() {
                    options.iter().rev().copied().collect::<Vec<_>>()
                } else {
                    options.to_vec()
                };
                for (option, label) in options {
                    let selected = *value == option;
                    let button = egui::Button::selectable(
                        selected,
                        RichText::new(label)
                            .size(11.5)
                            .family(theme::semibold())
                            .color(if selected { TEXT } else { TEXT_DIM }),
                    )
                    .fill(if selected {
                        theme::RAISED
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(if selected {
                        Stroke::new(1.0, BORDER)
                    } else {
                        Stroke::NONE
                    })
                    .corner_radius(CornerRadius::same(5))
                    .min_size(Vec2::new(68.0, 29.0));
                    if ui.add(button).clicked() && !selected {
                        *value = option;
                        *changed = true;
                    }
                }
            });
        });
}

/// Two-option segmented control over a bool; `no` is shown first.
fn segmented_bool(ui: &mut egui::Ui, value: &mut bool, yes: &str, no: &str, changed: &mut bool) {
    let mut v = *value as i32;
    segmented(ui, &mut v, &[(0, no), (1, yes)], changed);
    *value = v == 1;
}

fn overlay(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    let frame = egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(
            POPOVER.r(),
            POPOVER.g(),
            POPOVER.b(),
            245,
        ))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .stroke(Stroke::new(1.0, white(0x1c)))
        .inner_margin(Margin::same(24));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(560.0, 260.0).min(rect.size()),
            ))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    frame.show(&mut child, add);
}

fn res_name(r: i32) -> &'static str {
    match r {
        1 => "360p",
        2 => "540p",
        3 => "720p",
        _ => "1080p",
    }
}

/// Debug-only screenshots for docs: with `BAKERY_PS_DEMO=1`, setting
/// `BAKERY_PS_SCREENSHOT=out.png` saves the window and exits. `BAKERY_PS_VIEW`
/// picks the screen: `settings`, `link`, `second` or `empty`.
#[cfg(debug_assertions)]
impl App {
    fn debug_capture(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static FRAME: AtomicU32 = AtomicU32::new(0);
        let Some(path) = std::env::var_os("BAKERY_PS_SCREENSHOT") else {
            return;
        };
        let frame = FRAME.fetch_add(1, Ordering::Relaxed);
        if frame == 1 {
            let rows = self.rows();
            match std::env::var("BAKERY_PS_VIEW").unwrap_or_default().as_str() {
                "settings" => {
                    self.page = Page::Settings;
                    self.settings_section = SettingsSection::Video;
                }
                "link" => self.register = rows.last().map(|r| r.link_dialog(false)),
                "second" => self.register = rows.first().map(|r| r.link_dialog(true)),
                _ => {}
            }
        }
        if frame == 40 {
            ctx.request_screenshot(move |image| {
                let [w, h] = image.size;
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(pixmap) = resvg::tiny_skia::IntSize::from_wh(w as u32, h as u32)
                    .and_then(|size| resvg::tiny_skia::Pixmap::from_vec(bytes, size))
                {
                    let _ = pixmap.save_png(&path);
                }
                std::process::exit(0);
            });
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(debug_assertions)]
        self.debug_capture(ui.ctx());
        if self.streaming.is_some() {
            self.stream_ui(ui);
        } else {
            self.home_ui(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_modes_share_the_main_link_until_a_second_account_is_linked() {
        let main = SavedHost {
            nickname: "Console".into(),
            mac: "001122334455".into(),
            ps5: true,
            last_addr: "192.0.2.84".into(),
            regist_key: String::new(),
            rp_key: String::new(),
            account_id: Some([1; 8]),
        };
        let mut row = HostRow {
            name: main.nickname.clone(),
            addr: main.last_addr.clone(),
            ps5: main.ps5,
            awake: true,
            standby: false,
            seen: true,
            running_app: String::new(),
            saved: Some(main.clone()),
            watch_saved: None,
        };
        assert_eq!(row.stream_link(false).unwrap().account_id, Some([1; 8]));
        assert_eq!(row.stream_link(true).unwrap().account_id, Some([1; 8]));
        let setup = row.link_dialog(false);
        assert!(!setup.watch_only);

        let next = row.link_dialog(true);
        assert_eq!(next.addr, main.last_addr);
        assert_eq!(next.expected_mac.as_deref(), Some(main.mac.as_str()));
        assert!(next.watch_only && !next.manual);
        assert!(next.account_input.is_empty() && next.pin.is_empty());
        assert!(!can_link_console(&next, Some([1; 8])));

        row.watch_saved = Some(SavedHost {
            account_id: Some([2; 8]),
            ..main
        });
        assert_eq!(row.stream_link(false).unwrap().account_id, Some([1; 8]));
        assert_eq!(row.stream_link(true).unwrap().account_id, Some([2; 8]));
        row.saved = None;
        assert!(row.stream_link(false).is_none());
        assert_eq!(row.stream_link(true).unwrap().account_id, Some([2; 8]));
    }

    #[test]
    fn registration_checks_fields_and_uses_the_account_for_the_selected_mode() {
        let mut dlg = new_register(" console.local ".into(), true, true, false);
        dlg.pin = "01234567".into();
        assert!(can_link_console(&dlg, Some([1; 8])));
        assert!(!can_link_console(&dlg, None));
        dlg.account_input = "AgICAgICAgI=".into();
        assert!(can_link_console(&dlg, None));
        assert_eq!(registration_account(&dlg, Some([1; 8])), Ok([1; 8]));
        dlg.watch_only = true;
        assert_eq!(registration_account(&dlg, Some([1; 8])), Ok([2; 8]));
        dlg.addr = "https://console.local".into();
        assert!(!can_link_console(&dlg, Some([1; 8])));
    }

    #[test]
    fn registration_help_opens_the_guide_without_clearing_the_form() {
        let ctx = egui::Context::default();
        let mut dlg = new_register("192.0.2.84".into(), true, true, false);
        dlg.pin = "01234567".into();
        let mut action = RegistrationAction::None;
        let mut frame = |time, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| action = registration_form(ui.ctx(), &mut dlg, None, false),
            );
            output.textures_delta.clear();
            output.shapes
        };
        frame(0.0, vec![]);
        let pos = frame(1.0, vec![])
            .into_iter()
            .find_map(|shape| match shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "How to find it" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("account help link should be rendered");
        let button = |pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        frame(1.1, vec![egui::Event::PointerMoved(pos), button(true)]);
        frame(1.2, vec![button(false)]);
        assert!(action == RegistrationAction::Guide);
        assert_eq!(dlg.addr, "192.0.2.84");
        assert_eq!(dlg.pin, "01234567");
    }

    #[test]
    fn streamer_mode_masks_manual_and_discovered_addresses_and_connection_errors() {
        let address = "192.0.2.84";
        for manual in [false, true] {
            let render = |streamer_mode| {
                let ctx = egui::Context::default();
                let mut dlg = new_register(address.into(), true, manual, false);
                dlg.pin = "87654321".into();
                dlg.error = Some(format!("Could not reach {address}"));
                let mut first_frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                    registration_form(ui.ctx(), &mut dlg, Some([1; 8]), streamer_mode);
                });
                first_frame.textures_delta.clear();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        time: Some(1.0),
                        ..Default::default()
                    },
                    |ui| {
                        registration_form(ui.ctx(), &mut dlg, Some([1; 8]), streamer_mode);
                    },
                );
                output.textures_delta.clear();
                output
                    .shapes
                    .into_iter()
                    .filter_map(|shape| match shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let normal = render(false);
            assert!(normal.matches(address).count() >= 2, "{normal}");
            assert!(normal.contains("87654321"));
            let shared = render(true);
            assert!(shared.contains("Check the console address"));
            assert!(!shared.contains(address));
            assert!(!shared.contains("87654321"));
            if !manual {
                assert!(shared.contains("Address hidden"));
            }
        }
    }

    #[test]
    fn streamer_mode_hides_console_tile_and_log_details() {
        let row = HostRow {
            name: "Private console name".into(),
            addr: "192.0.2.42".into(),
            ps5: true,
            awake: true,
            standby: false,
            seen: true,
            running_app: String::new(),
            saved: Some(SavedHost {
                nickname: "Private console name".into(),
                mac: "000000000000".into(),
                ps5: true,
                last_addr: "192.0.2.42".into(),
                regist_key: String::new(),
                rp_key: String::new(),
                account_id: None,
            }),
            watch_saved: None,
        };
        let render = |streamer_mode| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                console_tile(ui, &row, 0, true, streamer_mode);
                log_page(ui, streamer_mode);
            });
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let normal = render(false).join("\n");
        assert!(normal.contains("Private console name"));
        let shared = render(true).join("\n");
        assert!(shared.contains("Console 1"));
        assert!(shared.contains("Logs are hidden"));
        assert!(!shared.contains("Private console name"));
    }
}
