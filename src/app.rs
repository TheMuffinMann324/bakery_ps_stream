//! The UI: home (console hero + console tiles), registration, settings and
//! the streaming view. Colours and type follow Bakery 8.3.0 (see theme.rs).

use crate::config::{Config, SavedHost, decode_account_id};
use crate::input::{self, Input};
use crate::logo::Logo;
use crate::remote::{self, Discovery, Registration, Session, SessionStatus, StreamOptions};
use crate::theme::{self, BG, BORDER, Icon, LIVE, PANEL, POPOVER, STANDBY, TEXT, TEXT_DIM, white};
use crate::video::VideoRenderer;
use eframe::egui::{self, Color32, CornerRadius, Key, Margin, RichText, Stroke, StrokeKind, Vec2};
use std::time::Duration;

const RAIL_WIDTH: f32 = 64.0;
const STREAM_TOOLBAR_HEIGHT: f32 = 52.0;
const PSN_ACCOUNT_GUIDE: &str =
    "https://chiaki-ng.github.io/chiaki-ng/setup/configuration/#registering-your-playstation";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Consoles,
    Settings,
    Log,
}

struct RegisterDialog {
    addr: String,
    ps5: bool,
    /// Opened from "Add console": the address is editable.
    manual: bool,
    watch_only: bool,
    watch_account: String,
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
    unsupported_format_logged: bool,
}

pub struct App {
    config: Config,
    discovery: Option<Discovery>,
    video: VideoRenderer,
    input: Input,
    page: Page,
    register: Option<RegisterDialog>,
    streaming: Option<Streaming>,
    banner: Option<(String, bool)>, // (message, is_error)
    fullscreen: bool,
    /// Animated B-snap mark at the top of the rail.
    logo: Logo,
    /// Console shown in the home hero, keyed by `HostRow::key`.
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

    fn display_name(&self, index: usize, streamer_mode: bool) -> String {
        if streamer_mode {
            format!("Console {}", index + 1)
        } else {
            self.name.clone()
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
    Wake,
    Forget,
    Register,
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
        let page = if config.settings.account_id_bytes().is_err() {
            Page::Settings
        } else {
            Page::Consoles
        };
        Ok(Self {
            config,
            discovery,
            video: VideoRenderer::new(rs),
            input: Input::new(),
            page,
            register: None,
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
        rows
    }

    fn connect(&mut self, ctx: &egui::Context, row: &HostRow) {
        let watch_only = self.config.settings.watch_only;
        let Some(saved) = row.saved_for(watch_only) else {
            self.register = Some(new_register(row.addr.clone(), row.ps5, false, watch_only));
            return;
        };
        let account = if watch_only {
            saved
                .account_id
                .ok_or_else(|| "Link this console again for Watch only.".to_owned())
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
                if watch_only {
                    let mut dialog = new_register(row.addr.clone(), row.ps5, false, true);
                    dialog.error = Some(e);
                    self.register = Some(dialog);
                } else {
                    self.banner = Some((format!("{e}. Add it under Settings."), true));
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
            HostAction::Play => self.connect(ctx, row),
            HostAction::Wake => {
                let saved = row
                    .saved_for(self.config.settings.watch_only)
                    .or_else(|| row.link())
                    .and_then(|s| s.to_registered());
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
                self.config
                    .hosts_mut(self.config.settings.watch_only)
                    .retain(|h| h.mac != key);
                self.config.save();
            }
            HostAction::Register => {
                self.register = Some(new_register(
                    row.addr.clone(),
                    row.ps5,
                    false,
                    self.config.settings.watch_only,
                ))
            }
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
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 22,
                bottom: 24,
            }))
            .show(ui, |ui| {
                // Rail border (border-right: 1px solid var(--border)).
                let r = ui.max_rect();
                ui.painter().vline(
                    r.left() - 28.0,
                    (r.top() - 22.0)..=(r.bottom() + 24.0),
                    Stroke::new(1.0, BORDER),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Consoles => self.consoles_page(ui, &ctx, &rows),
                        Page::Settings => self.settings_page(ui),
                        Page::Log => log_page(ui, self.config.settings.streamer_mode),
                    });
            });

        self.register_dialog(&ctx);
    }

    fn rail_ui(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            self.logo.show(ui, 40.0).on_hover_text("PS Stream");
            ui.add_space(15.0);
            theme::divider(ui);
            ui.add_space(10.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            for (page, icon, tip) in [
                (Page::Consoles, Icon::Home, "Home"),
                (Page::Settings, Icon::Gear, "Settings"),
                (Page::Log, Icon::List, "Log"),
            ] {
                if theme::nav_item(ui, icon, tip, self.page == page).clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn consoles_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rows: &[HostRow]) {
        // Heading, clear of the floating window controls.
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                theme::page_heading(
                    ui,
                    "PS Stream",
                    "Remote Play",
                    "Stream your PlayStation to this PC",
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.discovery.is_some() {
                    theme::status_badge(ui, "Searching network", Some(LIVE));
                } else {
                    theme::status_badge(ui, "Discovery off", None);
                }
            });
        });

        let streamer_mode = self.config.settings.streamer_mode;
        if let Some((msg, is_err)) = &self.banner {
            let summary = if *is_err {
                "Console action failed."
            } else {
                "Console action completed."
            };
            theme::notice(ui, screen_text(streamer_mode, msg, summary), *is_err);
            ui.add_space(14.0);
        }

        // Pick the hero console: the user's choice, else a linked online one.
        let hero = self
            .selected
            .as_ref()
            .and_then(|k| rows.iter().position(|r| &r.key() == k))
            .or_else(|| rows.iter().position(|r| r.awake && r.link().is_some()))
            .or_else(|| rows.iter().position(|r| r.link().is_some()))
            .or_else(|| (!rows.is_empty()).then_some(0));

        let mut action = None;
        match hero {
            Some(i) => {
                if let Some(a) = self.hero_ui(ui, &rows[i], i) {
                    action = Some((i, a));
                }
            }
            None => self.empty_hero_ui(ui),
        }

        ui.add_space(26.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("YOUR CONSOLES")
                    .size(12.0)
                    .family(theme::extrabold())
                    .color(TEXT)
                    .extra_letter_spacing(1.3),
            );
            ui.label(
                RichText::new(format!("{}", rows.len()))
                    .size(11.0)
                    .color(TEXT_DIM),
            );
        });
        ui.add_space(10.0);

        // Tile row: every console, then "Add console".
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);
            for (i, row) in rows.iter().enumerate() {
                if console_tile(ui, row, i, Some(i) == hero, streamer_mode).clicked() {
                    self.selected = Some(row.key());
                }
            }
            if add_tile(ui).clicked() {
                self.register = Some(new_register(
                    String::new(),
                    true,
                    true,
                    self.config.settings.watch_only,
                ));
            }
        });

        if let Some((i, a)) = action {
            self.do_action(ctx, &rows[i], a);
        }
    }

    /// Large panel for the selected console: art on the left, details and the
    /// stream controls on the right.
    fn hero_ui(&mut self, ui: &mut egui::Ui, row: &HostRow, index: usize) -> Option<HostAction> {
        let mut action = None;
        let mut changed = false;
        let mut open_settings = false;
        let streamer_mode = self.config.settings.streamer_mode;
        theme::panel().show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.spacing_mut().item_spacing.x = 0.0;
            let stage_w = (width * 0.42).clamp(260.0, 460.0);
            let row_resp = ui.horizontal_top(|ui| {
                // Stage column; painted below once the row height is known.
                ui.allocate_exact_size(Vec2::new(stage_w, 300.0), egui::Sense::hover());

                ui.vertical(|ui| {
                    ui.set_width(width - stage_w);
                    egui::Frame::NONE.inner_margin(Margin { left: 30, right: 28, top: 28, bottom: 26 }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                        let linked = row.link().is_some();
                        let (eyebrow, color) = if !linked {
                            ("New console found", None)
                        } else if row.awake {
                            ("Ready to stream", Some(LIVE))
                        } else if row.standby {
                            ("In rest mode", Some(STANDBY))
                        } else {
                            ("Not found on network", None)
                        };
                        theme::status_badge(ui, eyebrow, color);
                        ui.add_space(4.0);
                        ui.label(RichText::new(row.display_name(index, streamer_mode).to_uppercase()).size(30.0).family(theme::black()).color(TEXT).extra_letter_spacing(-0.6));
                        let model = if row.ps5 { "PlayStation 5" } else { "PlayStation 4" };
                        let address = screen_text(streamer_mode, &row.addr, "Address hidden");
                        ui.label(
                            RichText::new(format!("{model} · {address}"))
                                .size(11.5)
                                .color(TEXT_DIM),
                        );
                        if !streamer_mode && !row.running_app.is_empty() {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("NOW PLAYING").size(9.5).family(theme::extrabold()).color(white(0x70)).extra_letter_spacing(1.0));
                                ui.label(RichText::new(&row.running_app).size(12.0).family(theme::semibold()).color(TEXT));
                            });
                        }
                        ui.add_space(14.0);

                        let s = &mut self.config.settings;
                        segmented_bool(ui, &mut s.watch_only, "Watch only", "Play", &mut changed);
                        let hint = if s.watch_only {
                            "Stream with a second PSN account. Use your main account with the controller on the console."
                        } else {
                            "Use a controller or keyboard connected to this PC."
                        };
                        ui.label(RichText::new(hint).size(11.0).color(TEXT_DIM));
                        let mode_linked = row.saved_for(s.watch_only).is_some();
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let can_play = !mode_linked || row.awake || !row.seen;
                            let label = match (mode_linked, s.watch_only) {
                                (true, true) => "Watch stream",
                                (true, false) => "Start stream",
                                (false, true) => "Link for Watch only",
                                (false, false) => "Link console",
                            };
                            let start = ui.add_enabled_ui(can_play, |ui| theme::big_button(ui, Icon::Play, label)).inner;
                            if start.clicked() {
                                action = Some(if mode_linked { HostAction::Play } else { HostAction::Register });
                            }
                            if linked && row.standby && theme::icon_button(ui, Icon::Power, "Wake console", false).clicked() {
                                action = Some(HostAction::Wake);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if mode_linked && ui.add(theme::ghost_button("Forget")).on_hover_text("Remove the saved link for this mode").clicked() {
                                    action = Some(HostAction::Forget);
                                }
                            });
                        });
                        ui.add_space(14.0);
                        // Stream quality at a glance; click to change.
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let chips = [
                                res_name(s.resolution).to_uppercase(),
                                format!("{} FPS", s.fps),
                                if s.codec == 0 { "H.264".into() } else { "H.265".into() },
                                if s.hw_decode { "GPU DECODE".into() } else { "CPU DECODE".into() },
                            ];
                            for c in chips {
                                open_settings |= chip(ui, &c).clicked();
                            }
                        });
                    });
                });
            });
            // Stage: soft spotlight with the controller mark, full card height.
            let r = row_resp.response.rect;
            let stage = egui::Rect::from_min_size(r.min, Vec2::new(stage_w, r.height()));
            paint_stage(ui.painter(), stage, row);
        });
        if changed {
            self.config.save();
        }
        if open_settings {
            self.page = Page::Settings;
        }
        action
    }

    fn empty_hero_ui(&mut self, ui: &mut egui::Ui) {
        theme::panel().show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 300.0), egui::Sense::hover());
            paint_glow(
                &ui.painter().with_clip_rect(rect.shrink(1.0)),
                rect.center() - Vec2::new(0.0, 56.0),
                120.0,
            );
            theme::paint_icon(
                ui.painter(),
                Icon::Gamepad,
                rect.center() - Vec2::new(0.0, 62.0),
                64.0,
                white(0xd0),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_center_size(
                        rect.center() + Vec2::new(0.0, 74.0),
                        Vec2::new(560.0, 120.0),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            child.label(
                RichText::new("LOOKING FOR YOUR PLAYSTATION")
                    .size(14.0)
                    .family(theme::black())
                    .extra_letter_spacing(0.6),
            );
            child.label(
                RichText::new(
                    "Turn the console on or put it in rest mode, on the same network as this PC.",
                )
                .size(11.5)
                .color(TEXT_DIM),
            );
            child.add_space(8.0);
            if theme::icon_button(&mut child, Icon::Plus, "Add by IP address", false).clicked() {
                self.register = Some(new_register(
                    String::new(),
                    true,
                    true,
                    self.config.settings.watch_only,
                ));
            }
            // Searching indicator under the controller mark.
            let spin = egui::Rect::from_center_size(
                rect.center() - Vec2::new(0.0, 6.0),
                Vec2::splat(14.0),
            );
            ui.put(spin, egui::Spinner::new().size(14.0).color(TEXT_DIM));
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        theme::page_heading(
            ui,
            "PS Stream / Settings",
            "Settings",
            "Account, stream quality and controls",
        );
        let mut changed = false;
        let s = &mut self.config.settings;

        theme::panel().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            settings_row(ui, Icon::Monitor, "Streamer mode", "Hide console details, pairing fields and logs while sharing your screen.", |ui| {
                changed |= theme::switch(ui, &mut s.streamer_mode).changed();
            }, |_| {});
            if let Some((message, true)) = &self.banner {
                theme::notice(ui, screen_text(s.streamer_mode, message, "Settings or console action failed."), true);
            }

            let (id_txt, id_col) = match s.account_id_bytes() {
                Ok(_) => ("Valid account ID".to_owned(), Color32::from_rgb(0xb5, 0xd4, 0xc3)),
                Err(e) if !s.psn_account_id.is_empty() => (e, theme::DESTRUCTIVE_TEXT),
                Err(_) => ("Not set".to_owned(), TEXT_DIM),
            };
            let keyboard_on = s.keyboard_controls;
            settings_row(
                ui,
                Icon::User,
                "PSN account ID",
                "Use your main account's base64 ID to link a console for Play.",
                |ui| {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut s.psn_account_id).password(true)
                                .hint_text("AbCdEfGhIjk=")
                                .desired_width(220.0)
                                .margin(Margin::symmetric(10, 9)),
                        )
                        .changed();
                },
                |ui| {
                    ui.label(RichText::new(id_txt).size(11.0).color(id_col));
                    ui.hyperlink_to(
                        RichText::new("How to find it").size(11.0),
                        PSN_ACCOUNT_GUIDE,
                    );
                },
            );
            settings_row(ui, Icon::Monitor, "Resolution", "1080p needs a PS5 or PS4 Pro. Lower it on weak Wi-Fi.", |ui| {
                egui::ComboBox::new("res", "").width(140.0).selected_text(res_name(s.resolution)).show_ui(ui, |ui| {
                    for r in [4, 3, 2, 1] {
                        changed |= ui.selectable_value(&mut s.resolution, r, res_name(r)).changed();
                    }
                });
            }, |_| {});
            settings_row(ui, Icon::Play, "Frame rate and codec", "60 fps is smoother. H.265 looks better at the same bitrate.", |ui| {
                segmented(ui, &mut s.codec, &[(1, "H.265"), (0, "H.264")], &mut changed);
                ui.add_space(8.0);
                segmented(ui, &mut s.fps, &[(60, "60 FPS"), (30, "30 FPS")], &mut changed);
            }, |_| {});
            settings_row(ui, Icon::List, "Bitrate", "In kbps. 0 uses the console's default for the resolution.", |ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut s.bitrate_kbps).range(0..=100_000).speed(100).suffix(" kbps"))
                    .changed();
            }, |_| {});
            settings_row(ui, Icon::Gear, "Hardware decoding", "Decode video on the GPU (D3D11VA) for the lowest latency. Falls back to the CPU if unavailable.", |ui| {
                changed |= theme::switch(ui, &mut s.hw_decode).changed();
            }, |_| {});
            settings_row(ui, Icon::Gamepad, "Stream volume", "Adjust audio playback on this PC.", |ui| {
                changed |= volume_slider(ui, &mut s.volume_percent).changed();
            }, |_| {});
            settings_row(ui, Icon::Gamepad, "Stream mode", "Watch only uses a separate console link so you can play with the controller on your main account.", |ui| {
                segmented_bool(ui, &mut s.watch_only, "Watch only", "Play", &mut changed);
            }, |_| {});
            settings_row(ui, Icon::Gamepad, "Keyboard controls", "Play with the keyboard when no controller is connected.", |ui| {
                changed |= theme::switch(ui, &mut s.keyboard_controls).changed();
            }, |ui| {
                if keyboard_on {
                    ui.label(RichText::new(input::KEYBOARD_HELP).size(10.5).color(TEXT_DIM));
                }
            });
            settings_row(ui, Icon::Monitor, "Fullscreen on connect", "Start streams fullscreen. F11 toggles fullscreen and Escape returns to windowed mode.", |ui| {
                changed |= theme::switch(ui, &mut s.fullscreen_on_connect).changed();
            }, |_| {});
            settings_row_last(ui, Icon::Logo, "PS Stream", &format!("Version {} · Windows desktop · Built on chiaki-ng", env!("CARGO_PKG_VERSION")));
        });

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
                if dlg.watch_only {
                    let play_host = self.config.hosts.iter().find(|h| h.mac == saved.mac);
                    self.config.validate_watch_account(account, play_host)?;
                }
                Ok(saved)
            });
            match result {
                Ok(saved) => {
                    let next = if dlg.watch_only {
                        "Watch stream"
                    } else {
                        "Start stream"
                    };
                    self.banner = Some((
                        format!("Linked {}. Select {next} to connect.", saved.nickname),
                        false,
                    ));
                    self.selected = Some(saved.mac.clone());
                    self.config.upsert_host(saved, dlg.watch_only);
                    self.config.save();
                    self.register = None;
                    self.page = Page::Consoles;
                    return;
                }
                Err(e) => dlg.error = Some(e),
            }
        }

        let mut close = false;
        let mut start = false;
        let modal = egui::Modal::new(egui::Id::unique("register"))
            .backdrop_color(Color32::from_black_alpha(150))
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(0x1b, 0x1e, 0x25))
                    .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
                    .inner_margin(Margin::same(24))
                    .stroke(Stroke::new(1.0, white(0x1c))),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                theme::eyebrow(ui, if dlg.watch_only { "Watch only" } else { "Play" });
                ui.add_space(2.0);
                ui.label(RichText::new("LINK DEVICE").size(20.0).family(theme::black()).extra_letter_spacing(-0.4));
                ui.add_space(8.0);

                ui.add_enabled_ui(dlg.running.is_none(), |ui| if dlg.manual {
                    field_label(ui, "IP address");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut dlg.addr)
                                .password(streamer_mode)
                                .hint_text("192.168.1.50")
                                .desired_width(250.0)
                                .margin(Margin::symmetric(10, 9)),
                        );
                        ui.add_space(6.0);
                        let mut ps4 = !dlg.ps5;
                        segmented_bool(ui, &mut ps4, "PS4", "PS5", &mut false);
                        dlg.ps5 = !ps4;
                    });
                    ui.add_space(8.0);
                } else {
                    let model = if dlg.ps5 { "PS5" } else { "PS4" };
                    let address = screen_text(streamer_mode, &dlg.addr, "Address hidden");
                    ui.label(RichText::new(format!("{model} · {address}")).size(11.0).color(TEXT_DIM));
                    ui.add_space(4.0);
                });

                if dlg.watch_only {
                    field_label(ui, "Second PSN account ID");
                    ui.add_enabled(dlg.running.is_none(),
                        egui::TextEdit::singleline(&mut dlg.watch_account)
                            .password(true)
                            .hint_text("Base64 account ID")
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(10, 9)),
                    );
                    ui.hyperlink_to(
                        "How to find the account ID",
                        PSN_ACCOUNT_GUIDE,
                    );
                    ui.label(RichText::new("Sign in on the console with the second PSN account before getting the code. On PS5, enable Remote Play for that user.").size(11.0).color(TEXT_DIM));
                    ui.add_space(8.0);
                }

                ui.label(
                    RichText::new(if dlg.ps5 {
                        "On PS5, open Settings > System > Remote Play > Link Device and enter the 8-digit code."
                    } else {
                        "On PS4, open Settings > Remote Play Connection Settings > Add Device and enter the 8-digit code."
                    })
                        .size(11.0)
                        .color(TEXT_DIM),
                );
                ui.add_space(8.0);
                field_label(ui, "Code");
                let resp = ui.add_enabled(dlg.running.is_none(),
                    egui::TextEdit::singleline(&mut dlg.pin)
                        .password(streamer_mode)
                        .hint_text("00000000")
                        .char_limit(8)
                        .font(egui::FontId::new(22.0, theme::semibold()))
                        .align(egui::Align2::CENTER_CENTER)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 10)),
                );
                dlg.pin.retain(|c| c.is_ascii_digit());
                if let Some(e) = &dlg.error {
                    ui.add_space(6.0);
                    let summary = "Could not link. Check the account ID, address and pairing code.";
                    theme::notice(ui, screen_text(streamer_mode, e, summary), true);
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if dlg.running.is_some() {
                        ui.add(egui::Spinner::new().size(14.0).color(TEXT_DIM));
                        ui.label(RichText::new("Waiting for the console…").size(11.0).color(TEXT_DIM));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if dlg.running.is_none() {
                            let ready = dlg.pin.len() == 8 && !dlg.addr.trim().is_empty();
                            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            if ui.add_enabled(ready, theme::primary_button("Link")).clicked() || (ready && enter) {
                                start = true;
                            }
                        }
                        if ui.add(theme::outline_button("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() && dlg.running.is_none() {
            close = true;
        }

        if start {
            let dlg = self.register.as_mut().unwrap();
            dlg.error = None;
            dlg.addr = dlg.addr.trim().to_owned();
            let account = if dlg.watch_only {
                decode_account_id(&dlg.watch_account).and_then(|account| {
                    let play_host = self.config.hosts.iter().find(|h| h.last_addr == dlg.addr);
                    self.config.validate_watch_account(account, play_host)?;
                    Ok(account)
                })
            } else {
                self.config
                    .settings
                    .account_id_bytes()
                    .map_err(|e| format!("{e}. Set it under Settings first."))
            };
            match account {
                Err(e) => dlg.error = Some(e),
                Ok(account) => {
                    let pin = dlg.pin.parse::<u32>().unwrap_or(0);
                    match Registration::start(ctx.clone(), &dlg.addr, dlg.ps5, &account, pin) {
                        Ok(r) => dlg.running = Some((r, account)),
                        Err(e) => dlg.error = Some(format!("{e:#}")),
                    }
                }
            }
        }
        if close {
            self.register = None; // dropping cancels a running registration
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
                        |ui| stream_controls(ui, st, false, &mut toggle_fullscreen, &mut leave),
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
                                RichText::new(
                                    screen_text(streamer_mode, &st.title, "PlayStation")
                                        .to_uppercase(),
                                )
                                .size(20.0)
                                .family(theme::black()),
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
                                RichText::new("ENTER PASSCODE")
                                    .size(20.0)
                                    .family(theme::black()),
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
                                RichText::new("DISCONNECTED")
                                    .size(20.0)
                                    .family(theme::black()),
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
                        theme::eyebrow(ui, "Streaming");
                        let name = if streamer_mode {
                            "PLAYSTATION".to_owned()
                        } else {
                            st.session
                                .nickname()
                                .unwrap_or_else(|| st.title.clone())
                                .to_uppercase()
                        };
                        ui.label(RichText::new(name).size(20.0).family(theme::black()));
                        let pad = if watch_only {
                            "Use your main account with the controller on the console.".to_owned()
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
                            RichText::new("F1 TO CLOSE")
                                .size(9.5)
                                .family(theme::extrabold())
                                .color(white(0x60)),
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
    toggle_fullscreen: &mut bool,
    leave: &mut bool,
) {
    if ui.add(theme::danger_button("Disconnect")).clicked() {
        *leave = true;
    }
    if ui
        .add(theme::outline_button(if fullscreen {
            "Windowed"
        } else {
            "Fullscreen"
        }))
        .on_hover_text("F11")
        .clicked()
    {
        *toggle_fullscreen = true;
    }
    if ui
        .add(theme::outline_button(if stream.menu_open {
            "Close menu"
        } else {
            "Menu"
        }))
        .on_hover_text("F1")
        .clicked()
    {
        stream.menu_open = !stream.menu_open;
    }
}

fn new_register(addr: String, ps5: bool, manual: bool, watch_only: bool) -> RegisterDialog {
    RegisterDialog {
        addr,
        ps5,
        manual,
        watch_only,
        watch_account: String::new(),
        pin: String::new(),
        running: None,
        error: None,
    }
}

// ------------------------------------------------------------------ pieces

/// Hero art: a soft spotlight with a large controller mark and the console's state.
fn paint_stage(p: &egui::Painter, rect: egui::Rect, row: &HostRow) {
    let left = CornerRadius {
        nw: theme::RADIUS_CARD - 1,
        sw: theme::RADIUS_CARD - 1,
        ne: 0,
        se: 0,
    };
    p.rect_filled(rect, left, Color32::from_rgb(0x0c, 0x0d, 0x11));
    p.vline(rect.right(), rect.y_range(), Stroke::new(1.0, BORDER));
    let c = rect.center() - Vec2::new(0.0, 8.0);
    paint_glow(
        &p.with_clip_rect(rect),
        c,
        rect.width().min(rect.height()) * 0.55,
    );
    let fg = if row.awake { white(0xf0) } else { white(0x70) };
    theme::paint_icon(p, Icon::Gamepad, c, 96.0, fg);
    // Status light under the controller.
    let (_, color) = row.status();
    let dot = color.unwrap_or(Color32::from_rgb(0x73, 0x79, 0x85));
    let base = c + Vec2::new(0.0, 74.0);
    p.circle_filled(base, 9.0, theme::alpha(dot, 12));
    p.circle_filled(base, 3.5, dot);
}

/// Radial white glow made of stacked translucent circles.
fn paint_glow(p: &egui::Painter, center: egui::Pos2, radius: f32) {
    for i in (1..=10).rev() {
        let r = radius * i as f32 / 10.0;
        p.circle_filled(center, r, white(3));
    }
}

/// Compact console tile for the "Your consoles" row.
fn console_tile(
    ui: &mut egui::Ui,
    row: &HostRow,
    index: usize,
    selected: bool,
    streamer_mode: bool,
) -> egui::Response {
    let size = Vec2::new(214.0, 104.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    let border = if selected {
        white(0x60)
    } else if resp.hovered() {
        white(0x25)
    } else {
        BORDER
    };
    let fill = if selected { POPOVER } else { PANEL };
    p.rect(
        rect,
        CornerRadius::same(theme::RADIUS_CARD),
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let inner = rect.shrink(16.0);
    let (label, color) = row.status();
    let dot = color.unwrap_or(Color32::from_rgb(0x73, 0x79, 0x85));
    p.circle_filled(inner.left_top() + Vec2::new(3.0, 6.0), 3.0, dot);
    p.text(
        inner.left_top() + Vec2::new(12.0, 0.0),
        egui::Align2::LEFT_TOP,
        label.to_uppercase(),
        egui::FontId::new(9.0, theme::extrabold()),
        color.unwrap_or(TEXT_DIM),
    );
    theme::paint_icon(
        p,
        Icon::Gamepad,
        inner.right_top() + Vec2::new(-9.0, 6.0),
        18.0,
        white(0x60),
    );
    p.text(
        inner.left_top() + Vec2::new(0.0, 28.0),
        egui::Align2::LEFT_TOP,
        row.display_name(index, streamer_mode),
        egui::FontId::new(14.0, theme::semibold()),
        TEXT,
    );
    let sub = if row.link().is_none() {
        "Not linked"
    } else {
        screen_text(streamer_mode, &row.addr, "Address hidden")
    };
    p.text(
        inner.left_bottom(),
        egui::Align2::LEFT_BOTTOM,
        sub,
        egui::FontId::proportional(10.5),
        TEXT_DIM,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Dashed "Add console" tile.
fn add_tile(ui: &mut egui::Ui) -> egui::Response {
    let size = Vec2::new(214.0, 104.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(theme::RADIUS_CARD), white(0x05));
    }
    let col = if resp.hovered() {
        white(0x60)
    } else {
        white(0x25)
    };
    let r = rect.shrink(0.5);
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    p.extend(egui::Shape::dashed_line(
        &pts,
        Stroke::new(1.0, col),
        5.0,
        4.0,
    ));
    let fg = if resp.hovered() { TEXT } else { TEXT_DIM };
    theme::paint_icon(
        p,
        Icon::Plus,
        rect.center() - Vec2::new(0.0, 12.0),
        18.0,
        fg,
    );
    p.text(
        rect.center() + Vec2::new(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        "ADD CONSOLE",
        egui::FontId::new(10.0, theme::extrabold()),
        fg,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small outline chip (stream quality summary).
fn chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(9.5, theme::extrabold()),
        TEXT_DIM,
    );
    let (rect, resp) =
        ui.allocate_exact_size(galley.size() + Vec2::new(18.0, 10.0), egui::Sense::click());
    let p = ui.painter();
    let border = if resp.hovered() {
        white(0x30)
    } else {
        white(0x14)
    };
    p.rect(
        rect,
        CornerRadius::same(theme::RADIUS_INPUT),
        white(0x03),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    p.galley(rect.center() - galley.size() / 2.0, galley, TEXT_DIM);
    resp.on_hover_text("Change in Settings")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn log_page(ui: &mut egui::Ui, streamer_mode: bool) {
    theme::page_heading(
        ui,
        "PS Stream / Log",
        "Console & logs",
        "Output from the streaming engine",
    );
    if streamer_mode {
        theme::notice(ui, "Logs are hidden in Streamer mode.", false);
        return;
    }
    theme::panel().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        // Tab strip, like Bakery's "Live console" tab.
        egui::Frame::NONE
            .inner_margin(Margin {
                left: 18,
                right: 18,
                top: 12,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            theme::icon(ui, Icon::List, 13.0, TEXT);
                            ui.label(
                                RichText::new("Live console")
                                    .size(11.5)
                                    .family(theme::semibold()),
                            );
                        });
                        ui.add_space(9.0);
                        let r = ui.min_rect();
                        ui.painter()
                            .hline(r.x_range(), r.bottom(), Stroke::new(2.0, TEXT));
                    });
                });
            });
        theme::divider(ui);
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
fn settings_row(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    desc: &str,
    control: impl FnOnce(&mut egui::Ui),
    extra: impl FnOnce(&mut egui::Ui),
) {
    settings_row_inner(ui, icon, title, desc, Some(Box::new(control)), extra);
    theme::divider(ui);
}

fn settings_row_last(ui: &mut egui::Ui, icon: Icon, title: &str, desc: &str) {
    settings_row_inner(ui, icon, title, desc, None, |_| {});
}

#[allow(clippy::type_complexity)]
fn settings_row_inner(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    desc: &str,
    control: Option<Box<dyn FnOnce(&mut egui::Ui) + '_>>,
    extra: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::NONE
        .inner_margin(Margin::same(25))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(39.0), egui::Sense::hover());
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(6),
                    Stroke::new(1.0, BORDER),
                    StrokeKind::Inside,
                );
                theme::paint_icon(
                    ui.painter(),
                    icon,
                    rect.center(),
                    17.0,
                    Color32::from_rgb(0x9b, 0xa6, 0xb7),
                );
                let text_w = (ui.available_width() - 330.0).clamp(220.0, 570.0);
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 7.0;
                    ui.label(
                        RichText::new(title)
                            .size(14.0)
                            .family(theme::semibold())
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(desc)
                            .size(11.0)
                            .color(Color32::from_rgb(0x9b, 0xa6, 0xb7)),
                    );
                    extra(ui);
                });
                if let Some(control) = control {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
                }
            });
        });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(9.5)
            .family(theme::extrabold())
            .color(white(0x80))
            .extra_letter_spacing(1.0),
    );
    ui.add_space(-2.0);
}

/// Bakery tab list: dark track with the selected tab raised.
fn segmented(ui: &mut egui::Ui, value: &mut i32, options: &[(i32, &str)], changed: &mut bool) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(0x1c, 0x1f, 0x25))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS_BUTTON))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Called inside right_to_left layouts: add in reverse so they read left-to-right.
            let order: Vec<_> = if ui.layout().prefer_right_to_left() {
                options.iter().rev().collect()
            } else {
                options.iter().collect()
            };
            for &(v, label) in order {
                let sel = *value == v;
                let b = egui::Button::new(
                    RichText::new(label)
                        .size(10.5)
                        .family(theme::semibold())
                        .color(if sel { TEXT } else { TEXT_DIM }),
                )
                .fill(if sel { BG } else { Color32::TRANSPARENT })
                .stroke(if sel {
                    Stroke::new(1.0, white(0x14))
                } else {
                    Stroke::NONE
                })
                .corner_radius(CornerRadius::same(4))
                .min_size(Vec2::new(64.0, 26.0));
                if ui.add(b).clicked() && !sel {
                    *value = v;
                    *changed = true;
                }
            }
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        assert!(normal.contains("192.0.2.42"));
        let shared = render(true).join("\n");
        assert!(shared.contains("Console 1"));
        assert!(shared.contains("Logs are hidden"));
        assert!(!shared.contains("Private console name"));
        assert!(!shared.contains("192.0.2.42"));
    }
}
