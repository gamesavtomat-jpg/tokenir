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
    autobuy: bool, // ✅ NEW
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
    autobuy: bool, // ✅ NEW
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

enum AppEvent {
    UsersFetched(Vec<User>),
    UserAdded,
    UserRemoved,
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
            .with_min_inner_size([500.0, 400.0]),
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
    new_user_autobuy: bool, // ✅ NEW
    users: Vec<User>,
    status: String,
    is_loading: bool,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl AdminApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();

        let api_url =
            env::var("BACKEND_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());

        let admin_key = env::var("ADMIN_KEY").unwrap_or_default();

        let mut status = "Ready.".to_string();
        let mut is_loading = false;

        if !admin_key.is_empty() {
            status = "Auto-loading from .env...".to_string();
            is_loading = true;

            let tx_clone = tx.clone();
            let url = format!("{}/admin/users", api_url);
            let key = admin_key.clone();

            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let body = GetUsersReq { admin_key: key };

                match client.post(&url).json(&body).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(users) = resp.json::<Vec<User>>().await {
                            let _ = tx_clone.send(AppEvent::UsersFetched(users));
                        }
                    }
                    Ok(resp) => {
                        let _ = tx_clone.send(AppEvent::Error(resp.status().to_string()));
                    }
                    Err(e) => {
                        let _ = tx_clone.send(AppEvent::Error(e.to_string()));
                    }
                }
            });
        }

        Self {
            api_url,
            admin_key,
            new_user_key: String::new(),
            new_user_hint: String::new(),
            new_user_autobuy: true, // ✅ default true
            users: vec![],
            status,
            is_loading,
            tx,
            rx,
        }
    }

    fn fetch_users(&mut self) {
        self.is_loading = true;
        self.status = "Fetching users...".to_string();

        let tx = self.tx.clone();
        let url = format!("{}/admin/users", self.api_url);
        let body = GetUsersReq {
            admin_key: self.admin_key.clone(),
        };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(users) = resp.json::<Vec<User>>().await {
                        let _ = tx.send(AppEvent::UsersFetched(users));
                    }
                }
                Ok(resp) => {
                    let _ = tx.send(AppEvent::Error(resp.status().to_string()));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e.to_string()));
                }
            }
        });
    }

    fn add_user(&mut self) {
        if self.new_user_key.is_empty() {
            self.status = "Key cannot be empty".to_string();
            return;
        }

        self.is_loading = true;

        let tx = self.tx.clone();
        let url = format!("{}/admin/add_user", self.api_url);

        // ✅ Admin enforcement: always force autobuy ON for admins
        let autobuy = self.new_user_autobuy;

        let body = AddUserReq {
            admin_key: self.admin_key.clone(),
            payload: AddUserPayload {
                provided_key: self.new_user_key.clone(),
                hint: self.new_user_hint.clone(),
                autobuy,
            },
        };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::UserAdded);
                }
                Ok(resp) => {
                    let _ = tx.send(AppEvent::Error(resp.status().to_string()));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e.to_string()));
                }
            }
        });
    }

    fn remove_user(&mut self, user_id: i32) {
        self.is_loading = true;

        let tx = self.tx.clone();
        let url = format!("{}/admin/remove_user", self.api_url);
        let body = RemoveUserReq {
            admin_key: self.admin_key.clone(),
            user_id,
        };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::UserRemoved);
                }
                Ok(resp) => {
                    let _ = tx.send(AppEvent::Error(resp.status().to_string()));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e.to_string()));
                }
            }
        });
    }
}

impl eframe::App for AdminApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.rx.try_recv() {
            self.is_loading = false;
            match event {
                AppEvent::UsersFetched(users) => {
                    self.users = users;
                    self.status = "Users loaded".to_string();
                }
                AppEvent::UserAdded => {
                    self.new_user_key.clear();
                    self.new_user_hint.clear();
                    self.new_user_autobuy = true;
                    self.fetch_users();
                }
                AppEvent::UserRemoved => {
                    self.fetch_users();
                }
                AppEvent::Error(e) => {
                    self.status = format!("Error: {}", e);
                }
            }
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Admin Key:");
                ui.add(egui::TextEdit::singleline(&mut self.admin_key).password(true));
                ui.label("API:");
                ui.add(egui::TextEdit::singleline(&mut self.api_url));
                if ui.button("Refresh").clicked() {
                    self.fetch_users();
                }
                if self.is_loading {
                    ui.spinner();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.group(|ui| {
                ui.heading("Add New User");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_user_key).desired_width(180.0));
                    if ui.button("🎲").clicked() {
                        self.new_user_key = generate_random_string(32);
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.new_user_hint).desired_width(120.0));
                    ui.checkbox(&mut self.new_user_autobuy, "AutoBuy"); // ✅ NEW
                    if ui.button("➕ Add User").clicked() {
                        self.add_user();
                    }
                });
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("Existing Users");

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("users_grid")
                    .striped(true)
                    .spacing([20.0, 8.0])
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Key (Snippet)");
                        ui.strong("Hint");
                        ui.strong("Role");
                        ui.strong("AutoBuy");
                        ui.strong("Actions");
                        ui.end_row();

                        for user in self.users.clone() {
                            ui.label(user.id.to_string());

                            let key = user.access_key.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label(format!("{}...", &key[..8.min(key.len())]));
                                if ui.small_button("📋").clicked() {
                                    ui.output_mut(|o| o.copied_text = key);
                                }
                            });

                            ui.label(user.hint.clone().unwrap_or_default());

                            if user.admin {
                                ui.colored_label(egui::Color32::GREEN, "ADMIN");
                            } else {
                                ui.label("User");
                            }

                            // AutoBuy display
                            if user.admin {
                                ui.colored_label(egui::Color32::GREEN, "FORCED");
                            } else if user.autobuy {
                                ui.colored_label(egui::Color32::LIGHT_GREEN, "ON");
                            } else {
                                ui.colored_label(egui::Color32::GRAY, "OFF");
                            }

                            // Actions
                            if !user.admin {
                                if ui.button("🗑 Delete").clicked() {
                                    self.remove_user(user.id);
                                }
                            } else {
                                ui.label("-");
                            }

                            ui.end_row();
                        }
                    });
            });
        });

        egui::TopBottomPanel::bottom("status_panel").show(ctx, |ui| {
            ui.label(format!("Status: {}", self.status));
        });
    }
}
