#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
use dotenv::dotenv;
use std::env;

// --- Data Structures ---

#[derive(Clone, Debug, Deserialize)]
struct User {
    id: i32,
    access_key: Option<String>,
    hint: Option<String>,
    admin: bool,
    autobuy: bool,
}

#[derive(Serialize)]
struct AddUserReq {
    admin_key: String,
    payload: AddUserPayload,
}

#[derive(Serialize)]
struct AddUserPayload {
    provided_key: String,
    hint: String,
    autobuy: bool,
}

#[derive(Serialize)]
struct RemoveUserReq {
    admin_key: String,
    user_id: i32,
}

#[derive(Serialize)]
struct GetUsersReq {
    admin_key: String,
}

#[derive(Serialize)]
struct RestartReq {
    admin_key: String,
}

enum AppEvent {
    UsersFetched(Vec<User>),
    UserAdded,
    UserRemoved,
    ServerRestarted,
    Error(String),
}

fn generate_random_string(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    dotenv().ok();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([750.0, 500.0])
            .with_min_inner_size([400.0, 300.0]), // Lowered min size
        ..Default::default()
    };

    eframe::run_native(
        "Admin Panel",
        options,
        Box::new(|cc| Box::new(AdminApp::new(cc))),
    )
}

struct AdminApp {
    api_url: String,
    admin_key: String,
    new_user_key: String,
    new_user_hint: String,
    new_user_autobuy: bool,
    users: Vec<User>,
    status: String,
    is_loading: bool,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl AdminApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        let api_url = env::var("BACKEND_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
        let admin_key = env::var("ADMIN_KEY").unwrap_or_default();

        let mut app = Self {
            api_url,
            admin_key,
            new_user_key: String::new(),
            new_user_hint: String::new(),
            new_user_autobuy: true,
            users: vec![],
            status: "Ready.".to_string(),
            is_loading: false,
            tx,
            rx,
        };

        if !app.admin_key.is_empty() {
            app.fetch_users();
        }

        app
    }

    fn fetch_users(&mut self) {
        self.is_loading = true;
        self.status = "Fetching users...".to_string();
        let tx = self.tx.clone();
        let url = format!("{}/admin/users", self.api_url);
        let body = GetUsersReq { admin_key: self.admin_key.clone() };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(users) = resp.json::<Vec<User>>().await {
                        let _ = tx.send(AppEvent::UsersFetched(users));
                    }
                }
                Ok(resp) => { let _ = tx.send(AppEvent::Error(resp.status().to_string())); }
                Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
            }
        });
    }

    fn restart_server(&mut self) {
        self.is_loading = true;
        let tx = self.tx.clone();
        let url = format!("{}/admin/restart", self.api_url);
        let body = RestartReq { admin_key: self.admin_key.clone() };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => { let _ = tx.send(AppEvent::ServerRestarted); }
                Ok(resp) => { let _ = tx.send(AppEvent::Error(format!("Fail: {}", resp.status()))); }
                Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
            }
        });
    }

    fn add_user(&mut self) {
        if self.new_user_key.is_empty() { return; }
        self.is_loading = true;
        let tx = self.tx.clone();
        let url = format!("{}/admin/add_user", self.api_url);
        let body = AddUserReq {
            admin_key: self.admin_key.clone(),
            payload: AddUserPayload {
                provided_key: self.new_user_key.clone(),
                hint: self.new_user_hint.clone(),
                autobuy: self.new_user_autobuy,
            },
        };
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => { let _ = tx.send(AppEvent::UserAdded); }
                _ => { let _ = tx.send(AppEvent::Error("Failed to add".to_string())); }
            }
        });
    }

    fn remove_user(&mut self, user_id: i32) {
        self.is_loading = true;
        let tx = self.tx.clone();
        let url = format!("{}/admin/remove_user", self.api_url);
        let body = RemoveUserReq { admin_key: self.admin_key.clone(), user_id };
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => { let _ = tx.send(AppEvent::UserRemoved); }
                _ => { let _ = tx.send(AppEvent::Error("Failed to remove".to_string())); }
            }
        });
    }
}

impl eframe::App for AdminApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle events
        while let Ok(event) = self.rx.try_recv() {
            self.is_loading = false;
            match event {
                AppEvent::UsersFetched(users) => { self.users = users; self.status = "Users loaded".to_string(); }
                AppEvent::UserAdded => { self.new_user_key.clear(); self.fetch_users(); }
                AppEvent::UserRemoved => { self.fetch_users(); }
                AppEvent::ServerRestarted => { self.status = "Restarted!".to_string(); }
                AppEvent::Error(e) => { self.status = format!("Error: {}", e); }
            }
        }

        // --- TOP PANEL (Responsive Header) ---
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Admin Key:");
                ui.add(egui::TextEdit::singleline(&mut self.admin_key).password(true).desired_width(120.0));
                
                ui.label("API:");
                ui.add(egui::TextEdit::singleline(&mut self.api_url).desired_width(180.0));
                
                ui.separator();

                if ui.button("🔄 Refresh").clicked() { self.fetch_users(); }
                
                if ui.add(egui::Button::new("🔌 Restart Server").fill(egui::Color32::from_rgb(100, 40, 40))).clicked() {
                    self.restart_server();
                }

                if self.is_loading { ui.spinner(); }
            });
            ui.add_space(4.0);
        });

        // --- STATUS BAR ---
        egui::TopBottomPanel::bottom("status_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(format!("Status: {}", self.status));
            });
        });

        // --- MAIN CONTENT ---
        egui::CentralPanel::default().show(ctx, |ui| {
            // Responsive Add User Section
            ui.group(|ui| {
                ui.label(egui::RichText::new("Add New User").strong());
                ui.horizontal_wrapped(|ui| {
                    // Responsive field widths: use a fraction of available width but clamp it
                    let input_width = (ui.available_width() * 0.25).max(120.0);

                    ui.add(egui::TextEdit::singleline(&mut self.new_user_key)
                        .hint_text("Access Key")
                        .desired_width(input_width));
                    
                    if ui.button("🎲").on_hover_text("Generate Random Key").clicked() {
                        self.new_user_key = generate_random_string(32);
                    }

                    ui.add(egui::TextEdit::singleline(&mut self.new_user_hint)
                        .hint_text("Hint (Optional)")
                        .desired_width(input_width * 0.6));

                    ui.checkbox(&mut self.new_user_autobuy, "AutoBuy");

                    if ui.button("➕ Add User").clicked() {
                        self.add_user();
                    }
                });
            });

            ui.add_space(10.0);

            // Responsive Scrollable Grid
            ui.heading("Existing Users");
            ui.separator();

            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("users_grid")
                    .striped(true)
                    .spacing([15.0, 8.0])
                    .min_col_width(50.0)
                    .show(ui, |ui| {
                        // Headers
                        ui.strong("ID");
                        ui.strong("Key");
                        ui.strong("Hint");
                        ui.strong("Role");
                        ui.strong("AutoBuy");
                        ui.strong("Actions");
                        ui.end_row();

                        for user in self.users.clone() {
                            ui.label(user.id.to_string());

                            // Key snippet with copy button
                            ui.horizontal(|ui| {
                                let key = user.access_key.clone().unwrap_or_default();
                                ui.label(format!("{}...", &key[..4.min(key.len())]));
                                if ui.small_button("📋").clicked() {
                                    ui.output_mut(|o| o.copied_text = key);
                                }
                            });

                            ui.label(user.hint.clone().unwrap_or_default());

                            if user.admin {
                                ui.colored_label(egui::Color32::LIGHT_BLUE, "ADMIN");
                                ui.colored_label(egui::Color32::GRAY, "FORCED");
                                ui.label("-");
                            } else {
                                ui.label("User");
                                if user.autobuy {
                                    ui.colored_label(egui::Color32::LIGHT_GREEN, "ON");
                                } else {
                                    ui.label("OFF");
                                }
                                if ui.button("🗑").on_hover_text("Delete User").clicked() {
                                    self.remove_user(user.id);
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        });
    }
}