use eframe::egui;
use rand::{Rng, distributions::Alphanumeric, thread_rng};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
// Import dotenv and env
use dotenv::dotenv;
use std::env;

// --- Data Structures ---

#[derive(Clone, Debug, Deserialize)]
struct User {
    id: i32,
    access_key: Option<String>,
    hint: Option<String>,
    admin: bool,
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
    // 1. Load .env file at startup
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
    users: Vec<User>,
    status: String,
    is_loading: bool,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl AdminApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        
        // 2. Load Environment Variables with defaults
        let api_url = env::var("BACKEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
            
        let admin_key = env::var("ADMIN_KEY")
            .unwrap_or_default();

        let mut status = "Ready. Enter Key and Refresh.".to_string();
        let mut is_loading = false;

        // 3. Auto-Fetch: If keys exist in .env, start loading immediately
        if !admin_key.is_empty() {
            status = "Auto-loading from .env...".to_string();
            is_loading = true;
            
            let tx_clone = tx.clone();
            let url_clone = format!("{}/admin/users", api_url);
            let key_clone = admin_key.clone();

            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let body = GetUsersReq { admin_key: key_clone };
                
                match client.post(&url_clone).json(&body).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            if let Ok(users) = resp.json::<Vec<User>>().await {
                                let _ = tx_clone.send(AppEvent::UsersFetched(users));
                            } else {
                                let _ = tx_clone.send(AppEvent::Error("Failed to parse response".into()));
                            }
                        } else {
                            let _ = tx_clone.send(AppEvent::Error(format!("Server refused: {}", resp.status())));
                        }
                    }
                    Err(e) => {
                        let _ = tx_clone.send(AppEvent::Error(format!("Connection error: {}", e)));
                    }
                }
            });
        }
        
        Self {
            api_url,
            admin_key,
            new_user_key: "".to_string(),
            new_user_hint: "".to_string(),
            users: Vec::new(),
            status,
            is_loading,
            tx,
            rx,
        }
    }

    fn fetch_users(&mut self) {
        if self.admin_key.is_empty() {
            self.status = "⚠️ Please enter Admin Key first".to_string();
            return;
        }
        self.is_loading = true;
        self.status = "Fetching users...".to_string();
        
        let tx = self.tx.clone();
        let url = format!("{}/admin/users", self.api_url);
        let body = GetUsersReq { admin_key: self.admin_key.clone() };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        if let Ok(users) = resp.json::<Vec<User>>().await {
                            let _ = tx.send(AppEvent::UsersFetched(users));
                        } else {
                            let _ = tx.send(AppEvent::Error("Failed to parse response".into()));
                        }
                    } else {
                        let _ = tx.send(AppEvent::Error(format!("Server refused: {}", resp.status())));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Connection error: {}", e)));
                }
            }
        });
    }

    fn remove_user(&mut self, user_id: i32) {
        self.is_loading = true;
        self.status = format!("Removing user {}...", user_id);
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
                    let _ = tx.send(AppEvent::Error(format!("Failed to delete: {}", resp.status())));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e.to_string()));
                }
            }
        });
    }

    fn add_user(&mut self) {
        if self.new_user_key.is_empty() { 
            self.status = "⚠️ Key cannot be empty".to_string();
            return; 
        }
        
        self.is_loading = true;
        let tx = self.tx.clone();
        let url = format!("{}/admin/add_user", self.api_url);
        let body = AddUserReq {
            admin_key: self.admin_key.clone(),
            payload: AddUserPayload {
                provided_key: self.new_user_key.clone(),
                hint: self.new_user_hint.clone(),
            },
        };

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::UserAdded);
                }
                Ok(resp) => {
                    let _ = tx.send(AppEvent::Error(format!("Error: {}", resp.status())));
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
        // Handle Async Events
        while let Ok(event) = self.rx.try_recv() {
            self.is_loading = false;
            match event {
                AppEvent::UsersFetched(users) => {
                    self.users = users;
                    self.status = format!("✅ Successfully loaded {} users.", self.users.len());
                }
                AppEvent::UserAdded => {
                    self.status = "✅ User Added.".to_string();
                    self.new_user_key.clear();
                    self.new_user_hint.clear();
                    self.fetch_users(); 
                }
                AppEvent::UserRemoved => {
                    self.status = "🗑 User Removed.".to_string();
                    self.fetch_users(); 
                }
                AppEvent::Error(e) => {
                    self.status = format!("❌ Error: {}", e);
                }
            }
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Admin Key:");
                ui.add(egui::TextEdit::singleline(&mut self.admin_key).password(true).hint_text("Secret"));
                
                ui.label("API:");
                ui.add(egui::TextEdit::singleline(&mut self.api_url).hint_text("http://..."));

                ui.separator();

                if ui.button("🔄 Refresh List").clicked() {
                    self.fetch_users();
                }

                if self.is_loading {
                    ui.spinner();
                }
            });
            ui.add_space(5.0);
        });

        egui::TopBottomPanel::bottom("status_panel").show(ctx, |ui| {
            ui.label(format!("Status: {}", self.status));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.group(|ui| {
                ui.heading("Add New User");
                ui.horizontal(|ui| {
                    ui.label("Key:");
                    
                    // Allow editing, but also allow generation
                    ui.add(egui::TextEdit::singleline(&mut self.new_user_key).desired_width(200.0));
                    
                    // --- NEW FEATURE: RANDOM BUTTON ---
                    if ui.button("🎲 Random").on_hover_text("Generate 32-char key").clicked() {
                        self.new_user_key = generate_random_string(32);
                    }
                    // ----------------------------------

                    ui.label("Hint:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_user_hint).desired_width(100.0));
                    
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
                    .min_col_width(60.0)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Key (Snippet)");
                        ui.strong("Hint");
                        ui.strong("Role");
                        ui.strong("Actions");
                        ui.end_row();

                        let users_clone = self.users.clone(); 
                        
                        for user in users_clone {
                            ui.label(user.id.to_string());
                            
                            let key_display = user.access_key
                                .unwrap_or_default()
                                .chars()
                                .take(8)
                                .collect::<String>();
                            ui.label(format!("{}...", key_display));
                            ui.label(user.hint.unwrap_or_default());
                            
                            if user.admin {
                                ui.colored_label(egui::Color32::GREEN, "ADMIN");
                            } else {
                                ui.label("User");
                            }

                            if !user.admin {
                                if ui.add(
                                    egui::Button::new("🗑 Delete")
                                        .fill(egui::Color32::from_rgb(150, 50, 50))
                                ).clicked() 
                                {
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
    }
}