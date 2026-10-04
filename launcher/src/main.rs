#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod monitor;
mod process;

use anyhow::Result;
use config::{AppConfig, EnvPaths};
use gpui::*;
use monitor::{LogEntry, LogMonitor};
use process::ProcessManager;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let clean = path.trim_start_matches('/');
        let bytes: Option<&'static [u8]> = match clean {
            "icons/play-white.svg" => Some(include_bytes!("../ui/icons/play-white.svg")),
            "icons/stop-white.svg" => Some(include_bytes!("../ui/icons/stop-white.svg")),
            "icons/store-blue.svg" => Some(include_bytes!("../ui/icons/store-blue.svg")),
            "icons/store-gray.svg" => Some(include_bytes!("../ui/icons/store-gray.svg")),
            "icons/user-blue.svg" => Some(include_bytes!("../ui/icons/user-blue.svg")),
            "icons/user-gray.svg" => Some(include_bytes!("../ui/icons/user-gray.svg")),
            "icons/globe-white.svg" => Some(include_bytes!("../ui/icons/globe-white.svg")),
            "icons/lock-gray.svg" => Some(include_bytes!("../ui/icons/lock-gray.svg")),
            "icons/database-blue.svg" => Some(include_bytes!("../ui/icons/database-blue.svg")),
            "icons/copy-blue.svg" => Some(include_bytes!("../ui/icons/copy-blue.svg")),
            "icons/copy-gray.svg" => Some(include_bytes!("../ui/icons/copy-gray.svg")),
            "icons/trash-gray.svg" => Some(include_bytes!("../ui/icons/trash-gray.svg")),
            "icons/gear-gray.svg" => Some(include_bytes!("../ui/icons/gear-gray.svg")),
            "icons/folder-gray.svg" => Some(include_bytes!("../ui/icons/folder-gray.svg")),
            "preston.png" | "ui/preston.png" => Some(include_bytes!("../ui/preston.png")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ActiveTab {
    Dashboard,
    Logs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LogFilter {
    All,
    Nginx,
    Php,
    Db,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PortField {
    Web,
    Php,
    Db,
}

pub struct LauncherApp {
    paths: Arc<EnvPaths>,
    config: Arc<Mutex<AppConfig>>,
    pm: Arc<Mutex<ProcessManager>>,
    logs: Vec<LogEntry>,
    log_filter: LogFilter,
    active_tab: ActiveTab,
    auto_scroll: bool,

    is_running: bool,
    is_setup_mode: bool,
    is_busy: bool,
    admin_folder: Option<String>,
    toast: Option<(String, bool)>, // (message, is_error)

    show_settings: bool,
    show_reinstall_confirm: bool,
    input_web_port: String,
    input_php_port: String,
    input_db_port: String,
    focused_port: Option<PortField>,

    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
}

impl LauncherApp {
    pub fn new(
        paths: Arc<EnvPaths>,
        config: Arc<Mutex<AppConfig>>,
        pm: Arc<Mutex<ProcessManager>>,
        receiver: crossbeam_channel::Receiver<LogEntry>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (is_setup, admin) = paths.detect_prestashop_state();
        let (web_port, php_port, db_port) = {
            let cfg = config.lock().unwrap();
            (cfg.web_port, cfg.php_port, cfg.db_port)
        };

        // Start async background log polling
        cx.spawn(async move |this, cx| {
            loop {
                let mut entries = Vec::new();
                while let Ok(entry) = receiver.try_recv() {
                    entries.push(entry);
                }
                if !entries.is_empty() {
                    let res = this.update(cx, |this, cx| {
                        for entry in entries {
                            this.push_log(entry, cx);
                        }
                    });
                    if res.is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_millis(150)).await;
            }
        })
        .detach();

        Self {
            paths,
            config,
            pm,
            logs: Vec::new(),
            log_filter: LogFilter::All,
            active_tab: ActiveTab::Dashboard,
            auto_scroll: true,
            is_running: false,
            is_setup_mode: is_setup,
            is_busy: false,
            admin_folder: admin,
            toast: None,
            show_settings: false,
            show_reinstall_confirm: false,
            input_web_port: web_port.to_string(),
            input_php_port: php_port.to_string(),
            input_db_port: db_port.to_string(),
            focused_port: None,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
        }
    }

    fn refresh_status(&mut self) {
        let (is_setup, admin) = self.paths.detect_prestashop_state();
        let running = self.pm.lock().unwrap().is_running();
        self.is_running = running;
        self.is_setup_mode = is_setup;
        self.admin_folder = admin;
    }

    fn push_log(&mut self, entry: LogEntry, cx: &mut Context<Self>) {
        self.logs.push(entry);
        if self.logs.len() > 1000 {
            self.logs.remove(0);
        }
        if self.auto_scroll {
            self.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    fn start_services(&mut self, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.toast = None;
        cx.notify();

        let paths = self.paths.clone();
        let pm = self.pm.clone();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    paths.ensure_runtime_permissions();
                    let mut manager = pm.lock().unwrap();
                    manager.start_all(&paths)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.is_busy = false;
                match res {
                    Ok(_) => {
                        this.refresh_status();
                        this.toast = Some(("Services started successfully".into(), false));
                    }
                    Err(e) => {
                        this.refresh_status();
                        this.toast = Some((format!("Failed to start: {:#}", e), true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop_services(&mut self, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.toast = None;
        cx.notify();

        let pm = self.pm.clone();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let mut manager = pm.lock().unwrap();
                    manager.stop_all()
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.is_busy = false;
                match res {
                    Ok(_) => {
                        this.refresh_status();
                        this.toast = Some(("Services stopped".into(), false));
                    }
                    Err(e) => {
                        this.refresh_status();
                        this.toast = Some((format!("Failed to stop: {:#}", e), true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_shop(&self) {
        let (is_setup, _) = self.paths.detect_prestashop_state();
        let cfg = self.config.lock().unwrap();
        let url = if is_setup {
            format!("http://127.0.0.1:{}/install/", cfg.web_port)
        } else {
            format!("http://127.0.0.1:{}/", cfg.web_port)
        };
        let _ = open::that(&url);
    }

    fn open_admin(&self) {
        let (_, admin_folder) = self.paths.detect_prestashop_state();
        if let Some(admin) = admin_folder {
            let cfg = self.config.lock().unwrap();
            let url = format!("http://127.0.0.1:{}/{}/", cfg.web_port, admin);
            let _ = open::that(&url);
        }
    }

    fn open_logs_folder(&self) {
        let _ = open::that(&self.paths.logs_dir);
    }

    fn perform_reinstall(&mut self, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.show_reinstall_confirm = false;
        self.toast = Some(("Reinstalling and resetting PrestaShop...".into(), false));
        cx.notify();

        let paths = self.paths.clone();
        let pm = self.pm.clone();

        cx.spawn(async move |this, cx| {
            let res: anyhow::Result<()> = cx
                .background_executor()
                .spawn(async move {
                    let was_running = {
                        let mut manager = pm.lock().unwrap();
                        let running = manager.is_running();
                        if running {
                            let _ = manager.stop_all();
                        }
                        running
                    };

                    std::thread::sleep(Duration::from_millis(600));

                    let tmp_dir = &paths.tmp_dir;
                    let _ = std::fs::remove_file(tmp_dir.join("mysql.sock"));
                    let _ = std::fs::remove_file(tmp_dir.join("mariadb.pid"));
                    let _ = std::fs::remove_file(tmp_dir.join("nginx.pid"));
                    let _ = std::fs::remove_file(tmp_dir.join("php.pid"));

                    let sessions = tmp_dir.join("sessions");
                    if sessions.exists() {
                        let _ = std::fs::remove_dir_all(&sessions);
                        let _ = std::fs::create_dir_all(&sessions);
                    }

                    let uploads = tmp_dir.join("uploads");
                    if uploads.exists() {
                        let _ = std::fs::remove_dir_all(&uploads);
                        let _ = std::fs::create_dir_all(&uploads);
                    }

                    let db_dir = paths.data_dir.join("mariadb/prestashop");
                    if db_dir.exists() {
                        let _ = std::fs::remove_dir_all(&db_dir);
                    }

                    let app_dir = &paths.app_dir;
                    if app_dir.exists() {
                        let param_candidates = [
                            app_dir.join("app/config/parameters.php"),
                            app_dir.join("config/parameters.php"),
                            app_dir.join("app/config/parameters.yml"),
                        ];
                        for p in &param_candidates {
                            if p.exists() {
                                let _ = std::fs::remove_file(p);
                            }
                        }

                        let cache_dir = app_dir.join("var/cache");
                        if cache_dir.exists() {
                            let _ = std::fs::remove_dir_all(&cache_dir);
                            let _ = std::fs::create_dir_all(&cache_dir);
                        }

                        let var_logs = app_dir.join("var/logs");
                        if var_logs.exists() {
                            let _ = std::fs::remove_dir_all(&var_logs);
                            let _ = std::fs::create_dir_all(&var_logs);
                        }

                        let install_dir = app_dir.join("install");
                        if !install_dir.exists() {
                            for backup_name in
                                &["install_bak", "install.bak", "install_old", "install.old"]
                            {
                                let backup_path = app_dir.join(backup_name);
                                if backup_path.exists() {
                                    let _ = std::fs::rename(&backup_path, &install_dir);
                                    break;
                                }
                            }
                        }

                        if let Ok(entries) = std::fs::read_dir(app_dir) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                        if name.starts_with("admin")
                                            && name != "admin"
                                            && name != "admin-api"
                                            && name != "admin-dev"
                                        {
                                            let admin_dir = app_dir.join("admin");
                                            if !admin_dir.exists() {
                                                let _ = std::fs::rename(&path, &admin_dir);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    paths.ensure_runtime_permissions();

                    if was_running {
                        let mut manager = pm.lock().unwrap();
                        manager.start_all(&paths)?;
                    }

                    Ok(())
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.is_busy = false;
                match res {
                    Ok(_) => {
                        this.refresh_status();
                        this.toast =
                            Some(("PrestaShop reset complete. Ready for setup!".into(), false));
                    }
                    Err(e) => {
                        this.refresh_status();
                        this.toast = Some((format!("Reset failed: {:#}", e), true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let mut cfg = self.config.lock().unwrap();
        if let Ok(w) = self.input_web_port.parse::<u16>() {
            cfg.web_port = w;
        }
        if let Ok(p) = self.input_php_port.parse::<u16>() {
            cfg.php_port = p;
        }
        if let Ok(d) = self.input_db_port.parse::<u16>() {
            cfg.db_port = d;
        }
        self.pm.lock().unwrap().update_ports(&cfg);
        self.show_settings = false;
        self.toast = Some(("Network port settings saved".into(), false));
        cx.notify();
    }

    fn copy_db_credentials(&mut self, cx: &mut Context<Self>) {
        let db_port = { self.config.lock().unwrap().db_port };
        let info = format!(
            "Database Server: 127.0.0.1\nDatabase Port: {}\nDatabase Name: prestashop\nDatabase Login: root\nDatabase Password: (leave blank)",
            db_port
        );
        cx.write_to_clipboard(ClipboardItem::new_string(info));
        self.toast = Some(("Database credentials copied to clipboard!".into(), false));
        cx.notify();
    }

    fn copy_logs(&mut self, cx: &mut Context<Self>) {
        let text = self
            .logs
            .iter()
            .map(|l| l.formatted_line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.toast = Some(("Logs copied to clipboard!".into(), false));
        cx.notify();
    }
}

impl Render for LauncherApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_client_decorated = matches!(window.window_decorations(), Decorations::Client { .. });
        let is_maximized = window.is_maximized();

        let is_running = self.is_running;
        let is_setup = self.is_setup_mode;
        let is_busy = self.is_busy;
        let web_port = { self.config.lock().unwrap().web_port };
        let host_url = format!("http://127.0.0.1:{}", web_port);

        let app_bg = rgb(0xf8fafc);
        let card_bg = rgb(0xffffff);
        let border_color = rgb(0xe2e8f0);
        let text_main = rgb(0x0f172a);
        let text_muted = rgb(0x64748b);
        let primary_blue = rgb(0x2563eb);
        let primary_hover = rgb(0x1d4ed8);
        let success_green = rgb(0x16a34a);
        let danger_red = rgb(0xef4444);
        let danger_hover = rgb(0xdc2626);

        div()
            .id("app-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(app_bg)
            .text_color(text_main)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if this.show_settings {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.show_settings = false;
                            cx.notify();
                        }
                        "enter" => {
                            this.save_settings(cx);
                        }
                        "tab" => {
                            this.focused_port = match this.focused_port {
                                Some(PortField::Web) => Some(PortField::Php),
                                Some(PortField::Php) => Some(PortField::Db),
                                _ => Some(PortField::Web),
                            };
                            cx.notify();
                        }
                        "backspace" => {
                            let target = match this.focused_port {
                                Some(PortField::Web) => &mut this.input_web_port,
                                Some(PortField::Php) => &mut this.input_php_port,
                                Some(PortField::Db) => &mut this.input_db_port,
                                None => return,
                            };
                            target.pop();
                            cx.notify();
                        }
                        ch if ch.len() == 1 && ch.chars().next().unwrap().is_ascii_digit() => {
                            let target = match this.focused_port {
                                Some(PortField::Web) => &mut this.input_web_port,
                                Some(PortField::Php) => &mut this.input_php_port,
                                Some(PortField::Db) => &mut this.input_db_port,
                                None => return,
                            };
                            if target.len() < 5 {
                                target.push_str(ch);
                                cx.notify();
                            }
                        }
                        _ => {}
                    }
                }
            }))
            // 1. Header Bar (Native Window Titlebar & Controls)
            .child(
                div()
                    .id("window-titlebar")
                    .h(px(44.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(border_color)
                    .bg(card_bg)
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })
                    .on_mouse_down(MouseButton::Right, |event, window, _| {
                        window.show_window_menu(event.position);
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(26.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(border_color)
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(img("preston.png").size(px(20.0))),
                            )
                            .child(
                                div().font_weight(FontWeight::BOLD).text_sm().child(
                                    if self.active_tab == ActiveTab::Logs {
                                        "PrestaShop Portable — Log Monitor"
                                    } else {
                                        "PrestaShop Portable"
                                    },
                                ),
                            ),
                    )
                    // Right side: Window Controls & Version Badge
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("v{} (GPUI)", env!("CARGO_PKG_VERSION"))),
                            )
                            .children(if is_client_decorated {
                                Some(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        // Minimize Button
                                        .child(
                                            div()
                                                .id("win-ctrl-min")
                                                .cursor_pointer()
                                                .size(px(24.0))
                                                .rounded_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(rgb(0xe2e8f0)))
                                                .child(
                                                    div().w(px(10.0)).h(px(2.0)).bg(text_muted),
                                                )
                                                .on_click(cx.listener(|_, _, window, _| {
                                                    window.minimize_window();
                                                })),
                                        )
                                        // Maximize / Zoom Button
                                        .child(
                                            div()
                                                .id("win-ctrl-zoom")
                                                .cursor_pointer()
                                                .size(px(24.0))
                                                .rounded_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(rgb(0xe2e8f0)))
                                                .child(if is_maximized {
                                                    // Restore icon: two overlapping boxes
                                                    div()
                                                        .relative()
                                                        .size(px(10.0))
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .top_0()
                                                                .right_0()
                                                                .size(px(7.0))
                                                                .border_1()
                                                                .border_color(text_muted),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .bottom_0()
                                                                .left_0()
                                                                .size(px(7.0))
                                                                .border_1()
                                                                .border_color(text_muted)
                                                                .bg(card_bg),
                                                        )
                                                } else {
                                                    // Maximize icon: single box
                                                    div()
                                                        .size(px(9.0))
                                                        .border_1()
                                                        .border_color(text_muted)
                                                })
                                                .on_click(cx.listener(|_, _, window, _| {
                                                    window.zoom_window();
                                                })),
                                        )
                                        // Close Button
                                        .child(
                                            div()
                                                .id("win-ctrl-close")
                                                .cursor_pointer()
                                                .size(px(24.0))
                                                .rounded_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(danger_red).text_color(rgb(0xffffff)))
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(text_muted)
                                                .child("✕")
                                                .on_click(cx.listener(|this, _, _window, cx| {
                                                    let pm = this.pm.clone();
                                                    let _ = pm.lock().unwrap().stop_all();
                                                    cx.quit();
                                                })),
                                        ),
                                )
                            } else {
                                None
                            }),
                    ),
            )
            // 2. Notification Toast (if any)
            .children(self.toast.as_ref().map(|(msg, is_err)| {
                div()
                    .mx_4()
                    .mt_2()
                    .p_2()
                    .rounded_md()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .bg(if *is_err {
                        rgb(0xfef2f2)
                    } else {
                        rgb(0xf0fdf4)
                    })
                    .text_color(if *is_err { danger_red } else { success_green })
                    .border_1()
                    .border_color(if *is_err {
                        rgb(0xfecaca)
                    } else {
                        rgb(0xbbf7d0)
                    })
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(msg.clone())
                    .child(
                        div()
                            .id("toast-close")
                            .cursor_pointer()
                            .text_xs()
                            .p_1()
                            .child("×")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.toast = None;
                                cx.notify();
                            })),
                    )
            }))
            // 3. Main View Area
            .child(
                div()
                    .id("main-view-area")
                    .flex_1()
                    .p_4()
                    .overflow_y_scroll()
                    .child(match self.active_tab {
                        ActiveTab::Dashboard => self.render_dashboard(
                            is_running,
                            is_setup,
                            is_busy,
                            &host_url,
                            card_bg,
                            border_color,
                            text_muted,
                            text_main,
                            primary_blue,
                            primary_hover,
                            danger_red,
                            danger_hover,
                            success_green,
                            cx,
                        ),
                        ActiveTab::Logs => self.render_logs(
                            is_running,
                            &host_url,
                            card_bg,
                            border_color,
                            primary_blue,
                            text_muted,
                            success_green,
                            danger_red,
                            cx,
                        ),
                    }),
            )
            // 4. Bottom Footer Bar
            .child(
                div()
                    .h(px(40.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(border_color)
                    .bg(card_bg)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("footer-settings")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .hover(|s| s.bg(rgb(0xf1f5f9)))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/gear-gray.svg").size_3p5().text_color(text_muted))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(text_muted)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Settings"),
                                    )
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        let cfg = this.config.lock().unwrap();
                                        this.input_web_port = cfg.web_port.to_string();
                                        this.input_php_port = cfg.php_port.to_string();
                                        this.input_db_port = cfg.db_port.to_string();
                                        this.focused_port = Some(PortField::Web);
                                        this.show_settings = true;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("footer-reinstall")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .hover(|s| s.bg(rgb(0xfef2f2)))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/trash-gray.svg").size_3p5().text_color(danger_red))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(danger_red)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Reinstall"),
                                    )
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.show_reinstall_confirm = true;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("footer-logs-dir")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .hover(|s| s.bg(rgb(0xf1f5f9)))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/folder-gray.svg").size_3p5().text_color(text_muted))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(text_muted)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Logs Dir"),
                                    )
                                    .on_click(cx.listener(|this, _, _window, _cx| {
                                        this.open_logs_folder();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("footer-tab-toggle")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0xf1f5f9))
                            .hover(|s| s.bg(rgb(0xe2e8f0)))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text_main)
                            .child(if self.active_tab == ActiveTab::Dashboard {
                                "Show Logs ▼"
                            } else {
                                "Dashboard ▲"
                            })
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.active_tab = match this.active_tab {
                                    ActiveTab::Dashboard => ActiveTab::Logs,
                                    ActiveTab::Logs => ActiveTab::Dashboard,
                                };
                                cx.notify();
                            })),
                    ),
            )
            // 5. Settings Modal Overlay (conditional)
            .children(if self.show_settings {
                Some(self.render_settings_modal(border_color, card_bg, primary_blue, text_main, text_muted, danger_red, cx))
            } else {
                None
            })
            // 6. Reinstall Confirmation Modal (conditional)
            .children(if self.show_reinstall_confirm {
                Some(self.render_reinstall_modal(card_bg, border_color, danger_red, danger_hover, text_main, text_muted, cx))
            } else {
                None
            })
            // 7. Interactive Window Resize Handles (edges and corners for Wayland CSD)
            .children(if is_client_decorated && !is_maximized {
                Some(self.render_resize_handles())
            } else {
                None
            })
    }
}

impl LauncherApp {
    fn render_resize_handles(&self) -> impl IntoElement {
        let edge_size = px(5.0);
        let corner_size = px(8.0);

        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            // Top edge
            .child(
                div()
                    .id("resize-edge-top")
                    .absolute()
                    .top_0()
                    .left(corner_size)
                    .right(corner_size)
                    .h(edge_size)
                    .cursor(CursorStyle::ResizeUpDown)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::Top);
                    }),
            )
            // Bottom edge
            .child(
                div()
                    .id("resize-edge-bottom")
                    .absolute()
                    .bottom_0()
                    .left(corner_size)
                    .right(corner_size)
                    .h(edge_size)
                    .cursor(CursorStyle::ResizeUpDown)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::Bottom);
                    }),
            )
            // Left edge
            .child(
                div()
                    .id("resize-edge-left")
                    .absolute()
                    .left_0()
                    .top(corner_size)
                    .bottom(corner_size)
                    .w(edge_size)
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::Left);
                    }),
            )
            // Right edge
            .child(
                div()
                    .id("resize-edge-right")
                    .absolute()
                    .right_0()
                    .top(corner_size)
                    .bottom(corner_size)
                    .w(edge_size)
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::Right);
                    }),
            )
            // Top-left corner
            .child(
                div()
                    .id("resize-corner-tl")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(corner_size)
                    .cursor(CursorStyle::ResizeUpLeftDownRight)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::TopLeft);
                    }),
            )
            // Top-right corner
            .child(
                div()
                    .id("resize-corner-tr")
                    .absolute()
                    .top_0()
                    .right_0()
                    .size(corner_size)
                    .cursor(CursorStyle::ResizeUpRightDownLeft)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::TopRight);
                    }),
            )
            // Bottom-left corner
            .child(
                div()
                    .id("resize-corner-bl")
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .size(corner_size)
                    .cursor(CursorStyle::ResizeUpRightDownLeft)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::BottomLeft);
                    }),
            )
            // Bottom-right corner
            .child(
                div()
                    .id("resize-corner-br")
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .size(corner_size)
                    .cursor(CursorStyle::ResizeUpLeftDownRight)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_resize(ResizeEdge::BottomRight);
                    }),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_dashboard(
        &self,
        is_running: bool,
        is_setup: bool,
        is_busy: bool,
        host_url: &str,
        card_bg: Rgba,
        border_color: Rgba,
        text_muted: Rgba,
        text_main: Rgba,
        primary_blue: Rgba,
        primary_hover: Rgba,
        danger_red: Rgba,
        danger_hover: Rgba,
        success_green: Rgba,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            // Status Info Card
            .child(
                div()
                    .bg(card_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded_lg()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .text_xs()
                            .child(div().w(px(70.0)).text_color(text_muted).child("Status:"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(8.0))
                                            .rounded_full()
                                            .bg(if is_running {
                                                success_green
                                            } else if is_busy {
                                                rgb(0xf59e0b)
                                            } else {
                                                rgb(0x94a3b8)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if is_running {
                                                success_green
                                            } else if is_busy {
                                                rgb(0xf59e0b)
                                            } else {
                                                text_muted
                                            })
                                            .child(if is_running {
                                                "Running"
                                            } else if is_busy {
                                                "Processing..."
                                            } else {
                                                "Stopped"
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .text_xs()
                            .child(div().w(px(70.0)).text_color(text_muted).child("Host:"))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(text_main)
                                    .child(host_url.to_string()),
                            ),
                    )
                    .children(if is_running && !is_setup && self.admin_folder.is_some() {
                        Some(
                            div()
                                .flex()
                                .items_center()
                                .text_xs()
                                .child(div().w(px(70.0)).text_color(text_muted).child("Admin:"))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .child(format!("/{}/", self.admin_folder.as_ref().unwrap())),
                                        )
                                        .child(
                                            div()
                                                .text_color(success_green)
                                                .font_weight(FontWeight::BOLD)
                                                .text_xs()
                                                .child("(detected)"),
                                        ),
                                ),
                        )
                    } else {
                        None
                    }),
            )
            // Setup DB Credentials Card (when running & setup mode)
            .children(if is_running && is_setup {
                let db_port = { self.config.lock().unwrap().db_port };
                Some(
                    div()
                        .bg(rgb(0xf0f7ff))
                        .border_1()
                        .border_color(rgb(0xbfdbfe))
                        .rounded_lg()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(svg().path("icons/database-blue.svg").size_4().text_color(primary_blue))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .text_xs()
                                                .text_color(primary_blue)
                                                .child("Database Setup Credentials"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("btn-copy-db")
                                        .cursor_pointer()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_md()
                                        .bg(rgb(0xffffff))
                                        .border_1()
                                        .border_color(rgb(0xbfdbfe))
                                        .hover(|s| s.bg(rgb(0xe0f2fe)))
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(svg().path("icons/copy-blue.svg").size_3().text_color(primary_blue))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(primary_blue)
                                                .child("Copy Info"),
                                        )
                                        .on_click(cx.listener(|this, _, _window, cx| {
                                            this.copy_db_credentials(cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_xs()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(div().child("Host: 127.0.0.1"))
                                        .child(div().child("User: root"))
                                        .child(div().child("DB: prestashop")),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(div().child(format!("Port: {}", db_port)))
                                        .child(
                                            div()
                                                .text_color(text_muted)
                                                .child("Pass: (empty)"),
                                        ),
                                ),
                        ),
                )
            } else {
                None
            })
            // Big Primary Start/Stop Action Button
            .child(
                div()
                    .id("btn-main-action")
                    .h(px(52.0))
                    .w_full()
                    .rounded_lg()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .bg(if is_running {
                        danger_red
                    } else {
                        primary_blue
                    })
                    .hover(|s| {
                        s.bg(if is_running {
                            danger_hover
                        } else {
                            primary_hover
                        })
                    })
                    .child(
                        svg()
                            .path(if is_running {
                                "icons/stop-white.svg"
                            } else {
                                "icons/play-white.svg"
                            })
                            .size_5()
                            .text_color(rgb(0xffffff)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_sm()
                            .text_color(rgb(0xffffff))
                            .child(if is_busy {
                                "Please wait..."
                            } else if is_running {
                                "Stop Services"
                            } else {
                                "Start Services"
                            }),
                    )
                    .on_click(cx.listener(|this, _, _window, cx| {
                        if this.is_running {
                            this.stop_services(cx);
                        } else {
                            this.start_services(cx);
                        }
                    })),
            )
            // Secondary Buttons (Open Shop, Admin Login)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .w_full()
                    // Open Shop Button
                    .child(
                        div()
                            .id("btn-open-shop")
                            .flex_1()
                            .h(px(40.0))
                            .rounded_md()
                            .border_1()
                            .border_color(if !is_running {
                                border_color
                            } else if is_setup {
                                rgb(0x22c55e)
                            } else {
                                primary_blue
                            })
                            .bg(if !is_running {
                                rgb(0xf8fafc)
                            } else if is_setup {
                                rgb(0x16a34a)
                            } else {
                                card_bg
                            })
                            .hover(|s| {
                                if !is_running {
                                    s
                                } else {
                                    s.bg(if is_setup { rgb(0x15803d) } else { rgb(0xeff6ff) })
                                }
                            })
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(
                                svg()
                                    .path(if !is_running {
                                        "icons/store-gray.svg"
                                    } else if is_setup {
                                        "icons/globe-white.svg"
                                    } else {
                                        "icons/store-blue.svg"
                                    })
                                    .size_4()
                                    .text_color(if !is_running {
                                        text_muted
                                    } else if is_setup {
                                        rgb(0xffffff)
                                    } else {
                                        primary_blue
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if !is_running {
                                        text_muted
                                    } else if is_setup {
                                        rgb(0xffffff)
                                    } else {
                                        primary_blue
                                    })
                                    .child(if !is_running {
                                        "Open Shop (off)"
                                    } else if is_setup {
                                        "Start Shop Setup"
                                    } else {
                                        "Open Shop"
                                    }),
                            )
                            .on_click(cx.listener(|this, _, _window, _cx| {
                                if this.is_running {
                                    this.open_shop();
                                }
                            })),
                    )
                    // Admin Login Button
                    .child(
                        div()
                            .id("btn-open-admin")
                            .flex_1()
                            .h(px(40.0))
                            .rounded_md()
                            .border_1()
                            .border_color(if !is_running || is_setup || self.admin_folder.is_none() {
                                border_color
                            } else {
                                primary_blue
                            })
                            .bg(if !is_running || is_setup || self.admin_folder.is_none() {
                                rgb(0xf8fafc)
                            } else {
                                card_bg
                            })
                            .hover(|s| {
                                if !is_running || is_setup || self.admin_folder.is_none() {
                                    s
                                } else {
                                    s.bg(rgb(0xeff6ff))
                                }
                            })
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(
                                svg()
                                    .path(if !is_running || self.admin_folder.is_none() {
                                        "icons/user-gray.svg"
                                    } else if is_setup {
                                        "icons/lock-gray.svg"
                                    } else {
                                        "icons/user-blue.svg"
                                    })
                                    .size_4()
                                    .text_color(if !is_running || is_setup || self.admin_folder.is_none() {
                                        text_muted
                                    } else {
                                        primary_blue
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if !is_running || is_setup || self.admin_folder.is_none() {
                                        text_muted
                                    } else {
                                        primary_blue
                                    })
                                    .child(if !is_running {
                                        "Admin Login (off)"
                                    } else if is_setup {
                                        "Admin (Locked)"
                                    } else {
                                        "Admin Login"
                                    }),
                            )
                            .on_click(cx.listener(|this, _, _window, _cx| {
                                if this.is_running && !this.is_setup_mode {
                                    this.open_admin();
                                }
                            })),
                    ),
            )
            // Setup Mode Footnote
            .children(if is_running && is_setup {
                Some(
                    div()
                        .mt_1()
                        .p_2()
                        .rounded_md()
                        .bg(rgb(0xf8fafc))
                        .border_1()
                        .border_color(border_color)
                        .text_xs()
                        .text_color(text_muted)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().child("Complete setup in browser."))
                        .child(
                            div()
                                .id("link-reinstall")
                                .cursor_pointer()
                                .text_color(danger_red)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Reset & Reinstall")
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    this.show_reinstall_confirm = true;
                                    cx.notify();
                                })),
                        ),
                )
            } else {
                None
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_logs(
        &self,
        is_running: bool,
        host_url: &str,
        card_bg: Rgba,
        border_color: Rgba,
        primary_blue: Rgba,
        text_muted: Rgba,
        success_green: Rgba,
        danger_red: Rgba,
        cx: &mut Context<Self>,
    ) -> Div {
        let filtered_entries: Vec<&LogEntry> = self
            .logs
            .iter()
            .filter(|l| match self.log_filter {
                LogFilter::All => true,
                LogFilter::Nginx => l.source == "NGINX",
                LogFilter::Php => l.source == "PHP",
                LogFilter::Db => l.source == "DB",
            })
            .collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            // Compact Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_2()
                    .bg(card_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded_md()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .size(px(7.0))
                                            .rounded_full()
                                            .bg(if is_running { success_green } else { rgb(0x94a3b8) }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if is_running { success_green } else { text_muted })
                                            .child(if is_running { "Running" } else { "Stopped" }),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(host_url.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .children(if is_running {
                                Some(
                                    div()
                                        .id("btn-log-stop")
                                        .cursor_pointer()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_md()
                                        .bg(danger_red)
                                        .text_color(rgb(0xffffff))
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(svg().path("icons/stop-white.svg").size_3().text_color(rgb(0xffffff)))
                                        .child("Stop")
                                        .on_click(cx.listener(|this, _, _window, cx| {
                                            this.stop_services(cx);
                                        })),
                                )
                            } else {
                                None
                            })
                            .child(
                                div()
                                    .id("btn-log-open-shop")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(rgb(0xf1f5f9))
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(primary_blue)
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/store-blue.svg").size_3().text_color(primary_blue))
                                    .child("Open Shop")
                                    .on_click(cx.listener(|this, _, _window, _cx| {
                                        this.open_shop();
                                    })),
                            ),
                    ),
            )
            // Filter Bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(div().text_xs().text_color(text_muted).child("Filter:"))
                            .child(self.filter_chip(LogFilter::All, "All", "chip-all", cx))
                            .child(self.filter_chip(LogFilter::Nginx, "Nginx", "chip-nginx", cx))
                            .child(self.filter_chip(LogFilter::Php, "PHP", "chip-php", cx))
                            .child(self.filter_chip(LogFilter::Db, "DB", "chip-db", cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("toggle-auto-scroll")
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(if self.auto_scroll { "☑ Auto-scroll" } else { "☐ Auto-scroll" })
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.auto_scroll = !this.auto_scroll;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("btn-clear-logs")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(rgb(0xf1f5f9))
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/trash-gray.svg").size_3().text_color(text_muted))
                                    .child("Clear")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.logs.clear();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("btn-copy-logs")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(rgb(0xf1f5f9))
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/copy-gray.svg").size_3().text_color(text_muted))
                                    .child("Copy")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.copy_logs(cx);
                                    })),
                            ),
                    ),
            )
            // Dark Terminal Console
            .child(
                div()
                    .id("terminal-console")
                    .flex_1()
                    .bg(rgb(0x0f172a))
                    .border_1()
                    .border_color(rgb(0x1e293b))
                    .rounded_lg()
                    .p_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .children(if filtered_entries.is_empty() {
                        vec![div()
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child("Waiting for runtime logs...")]
                    } else {
                        filtered_entries
                            .iter()
                            .map(|entry| {
                                let (source_color, tag) = match entry.source.as_str() {
                                    "NGINX" => (rgb(0x38bdf8), "[NGINX]"),
                                    "PHP" => (rgb(0xc084fc), "[PHP]"),
                                    "DB" => (rgb(0x4ade80), "[DB]"),
                                    _ => (rgb(0xf43f5e), "[SYS]"),
                                };

                                div()
                                    .flex()
                                    .items_start()
                                    .gap_2()
                                    .text_xs()
                                    .child(
                                        div()
                                            .w(px(55.0))
                                            .text_color(source_color)
                                            .font_weight(FontWeight::BOLD)
                                            .child(tag),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_color(rgb(0xe2e8f0))
                                            .child(entry.formatted_line.clone()),
                                    )
                            })
                            .collect()
                    }),
            )
    }

    fn filter_chip(&self, filter: LogFilter, label: &'static str, id: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let is_selected = self.log_filter == filter;
        div()
            .id(id)
            .cursor_pointer()
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(if is_selected { rgb(0x2563eb) } else { rgb(0xf1f5f9) })
            .text_color(if is_selected { rgb(0xffffff) } else { rgb(0x64748b) })
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(label)
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.log_filter = filter;
                cx.notify();
            }))
    }

    fn render_settings_modal(
        &self,
        border_color: Rgba,
        card_bg: Rgba,
        primary_blue: Rgba,
        text_main: Rgba,
        text_muted: Rgba,
        danger_red: Rgba,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .bg(rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(380.0))
                    .bg(card_bg)
                    .rounded_xl()
                    .border_1()
                    .border_color(border_color)
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(svg().path("icons/gear-gray.svg").size_4().text_color(primary_blue))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_sm()
                                    .child("Network Port Settings"),
                            ),
                    )
                    // Input rows
                    .child(self.render_port_input(
                        "Web Port (Nginx):",
                        &self.input_web_port,
                        PortField::Web,
                        "port-web",
                        primary_blue,
                        border_color,
                        text_main,
                        text_muted,
                        cx,
                    ))
                    .child(self.render_port_input(
                        "PHP Port (FastCGI):",
                        &self.input_php_port,
                        PortField::Php,
                        "port-php",
                        primary_blue,
                        border_color,
                        text_main,
                        text_muted,
                        cx,
                    ))
                    .child(self.render_port_input(
                        "Database Port (MariaDB):",
                        &self.input_db_port,
                        PortField::Db,
                        "port-db",
                        primary_blue,
                        border_color,
                        text_main,
                        text_muted,
                        cx,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_muted)
                            .child("Click an input and type numbers. Press Tab to switch fields."),
                    )
                    // Troubleshooting & Reset
                    .child(
                        div()
                            .mt_1()
                            .pt_2()
                            .border_t_1()
                            .border_color(border_color)
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(danger_red)
                                    .child("Troubleshooting & Reset"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child("If setup failed or database is corrupted, reset to start fresh."),
                            )
                            .child(
                                div()
                                    .id("btn-modal-reinstall")
                                    .cursor_pointer()
                                    .px_3()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(rgb(0xfef2f2))
                                    .border_1()
                                    .border_color(rgb(0xfecaca))
                                    .hover(|s| s.bg(rgb(0xfee2e2)))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(svg().path("icons/trash-gray.svg").size_3p5().text_color(danger_red))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(danger_red)
                                            .child("Reset & Reinstall PrestaShop"),
                                    )
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.show_settings = false;
                                        this.show_reinstall_confirm = true;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // Modal actions
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_2()
                            .child(
                                div()
                                    .id("btn-cancel-settings")
                                    .cursor_pointer()
                                    .px_4()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(rgb(0xf1f5f9))
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Cancel")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.show_settings = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("btn-save-settings")
                                    .cursor_pointer()
                                    .px_4()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(primary_blue)
                                    .hover(|s| s.bg(rgb(0x1d4ed8)))
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xffffff))
                                    .child("Save Ports")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.save_settings(cx);
                                    })),
                            ),
                    ),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_port_input(
        &self,
        label: &'static str,
        value: &str,
        field: PortField,
        element_id: &'static str,
        primary_blue: Rgba,
        border_color: Rgba,
        text_main: Rgba,
        text_muted: Rgba,
        cx: &mut Context<Self>,
    ) -> Div {
        let is_focused = self.focused_port == Some(field);
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_xs().text_color(text_muted).child(label))
            .child(
                div()
                    .id(element_id)
                    .w(px(100.0))
                    .h(px(32.0))
                    .rounded_md()
                    .px_2()
                    .border_1()
                    .border_color(if is_focused { primary_blue } else { border_color })
                    .bg(if is_focused { rgb(0xffffff) } else { rgb(0xf8fafc) })
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(text_main)
                    .child(if value.is_empty() { "_".to_string() } else { value.to_string() })
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.focused_port = Some(field);
                        cx.notify();
                    })),
            )
    }

    fn render_reinstall_modal(
        &self,
        card_bg: Rgba,
        border_color: Rgba,
        danger_red: Rgba,
        danger_hover: Rgba,
        _text_main: Rgba,
        text_muted: Rgba,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .bg(rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(360.0))
                    .bg(card_bg)
                    .rounded_xl()
                    .border_1()
                    .border_color(border_color)
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(svg().path("icons/trash-gray.svg").size_4().text_color(danger_red))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_sm()
                                    .text_color(danger_red)
                                    .child("Reset & Reinstall PrestaShop?"),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_muted)
                            .child(
                                "This will cleanly wipe local database data, remove generated session files and cache, and restore the installation wizard. Use this if installation was interrupted or stuck.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_2()
                            .child(
                                div()
                                    .id("btn-cancel-reinstall")
                                    .cursor_pointer()
                                    .px_3()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(rgb(0xf1f5f9))
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Cancel")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.show_reinstall_confirm = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("btn-confirm-reinstall")
                                    .cursor_pointer()
                                    .px_3()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(danger_red)
                                    .hover(|s| s.bg(danger_hover))
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xffffff))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(svg().path("icons/trash-gray.svg").size_3().text_color(rgb(0xffffff)))
                                    .child("Confirm Reset")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.perform_reinstall(cx);
                                    })),
                            ),
                    ),
            )
    }
}

fn main() {
    let paths = Arc::new(EnvPaths::resolve().unwrap_or_else(|err| {
        eprintln!("Fatal: cannot resolve runtime paths: {}", err);
        panic!("Fatal: cannot resolve runtime paths");
    }));

    let config = Arc::new(Mutex::new(AppConfig::default()));
    let pm = Arc::new(Mutex::new(ProcessManager::new(&config.lock().unwrap())));

    // Start Log Monitor thread
    let log_monitor = LogMonitor::start(paths.logs_dir.clone());

    let pm_for_quit = pm.clone();

    Application::new()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            // Graceful shutdown on app quit
            let pm_quit = pm_for_quit.clone();
            cx.on_app_quit(move |_cx| {
                let mut manager = pm_quit.lock().unwrap();
                let _ = manager.stop_all();
                async {}
            })
            .detach();

            let window_bounds = Bounds::centered(None, size(px(460.0), px(600.0)), cx);
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("PrestaShop Portable".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(380.0), px(460.0))),
                is_resizable: true,
                ..Default::default()
            };

            let app_paths = paths.clone();
            let app_config = config.clone();
            let app_pm = pm.clone();
            let receiver = log_monitor.receiver.clone();

            let _ = cx
                .open_window(window_options, |_, cx| {
                    cx.new(|cx| LauncherApp::new(app_paths, app_config, app_pm, receiver, cx))
                })
                .expect("failed to open window");
        });
}
