use eframe::egui;
use egui::{Color32, FontId, RichText, ScrollArea};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use std::{
    fs::File,
    io::{Read, Write},
    str::FromStr,
    sync::{
        Arc,
        RwLock, // Added RwLock
        atomic::{AtomicI64, AtomicU64},
    },
};
use tokenir_ui::Token;
use tokio::sync::{Mutex, watch::Sender};

use crate::{
    autobuy::{AutoBuyConfig, BuyAutomata},
    blacklist::{self, Blacklist},
    filter::{FilterSet, Filters, Tag},
    pool::{self, Pool},
};

// ... [KeyConfig struct remains the same] ...
#[derive(Serialize, Deserialize)]
pub struct KeyConfig {
    pub access_key: String,
}

// ==============================================================================
// 2. LAUNCHER (State Manager)
// ==============================================================================

pub struct Launcher {
    state: AppState,

    pool: Arc<Mutex<Pool>>,
    blacklist: Arc<Mutex<Blacklist>>,
    price: Arc<AtomicU64>,
    total_token_count: Arc<AtomicI64>,
    automata: Arc<Mutex<BuyAutomata>>,
    config: Option<AutoBuyConfig>,

    startup_tx: Sender<String>,

    // Added permission lock
    is_logged_in: Arc<RwLock<bool>>,
    pub trade_terminal: Arc<RwLock<TradeTerminal>>,
}

enum AppState {
    Login {
        input_key: String,
        error_msg: Option<String>,
    },
    Running(MyApp),
}

impl Launcher {
    pub fn new(
        pool: Arc<Mutex<Pool>>,
        blacklist: Arc<Mutex<Blacklist>>,
        price: Arc<AtomicU64>,
        total: Arc<AtomicI64>,
        automata: Arc<Mutex<BuyAutomata>>,
        config: Option<AutoBuyConfig>,
        startup_tx: Sender<String>,
        is_logged_in: Arc<RwLock<bool>>, // New argument,
        trade_terminal: Arc<RwLock<TradeTerminal>>,
    ) -> Self {
        // 1. Try to load key.json
        let loaded_key = if let Ok(mut file) = File::open("key.json") {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                serde_json::from_str::<KeyConfig>(&content).ok()
            } else {
                None
            }
        } else {
            None
        };

        // 2. Determine initial state
        let state = if let Some(k) = loaded_key {
            // Key exists: Signal main thread
            let _ = startup_tx.send(k.access_key.clone());

            // ALLOW BROWSER
            if let Ok(mut guard) = is_logged_in.write() {
                *guard = true;
            }

            let app = MyApp::new(
                pool.clone(),
                blacklist.clone(),
                price.clone(),
                total.clone(),
                automata.clone(),
                config.clone(),
                trade_terminal.clone(),
            );
            AppState::Running(app)
        } else {
            // Key missing: Deny browser (default is false, but ensuring safety)
            if let Ok(mut guard) = is_logged_in.write() {
                *guard = false;
            }

            AppState::Login {
                input_key: String::new(),
                error_msg: None,
            }
        };

        Self {
            state,
            pool,
            blacklist,
            price,
            total_token_count: total,
            automata,
            config,
            startup_tx,
            is_logged_in,
            trade_terminal,
        }
    }
}

impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let mut next_state: Option<AppState> = None;

        match &mut self.state {
            // --- LOGIN SCREEN ---
            AppState::Login {
                input_key,
                error_msg,
            } => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.heading("Authentication Required");
                        ui.add_space(20.0);
                        ui.label("Please enter your ACCESS_KEY to continue:");

                        let text_res = ui.add(
                            egui::TextEdit::singleline(input_key)
                                .password(true)
                                .hint_text("Paste key here..."),
                        );

                        ui.add_space(10.0);

                        if let Some(msg) = error_msg {
                            ui.label(RichText::new(msg.clone()).color(Color32::RED));
                            ui.add_space(5.0);
                        }

                        let clicked = ui.button("Save & Enter").clicked();
                        let enter_pressed =
                            text_res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter));

                        if clicked || enter_pressed {
                            if input_key.trim().is_empty() {
                                *error_msg = Some("Key cannot be empty".to_string());
                            } else {
                                let key_val = input_key.trim().to_string();
                                let cfg = KeyConfig {
                                    access_key: key_val.clone(),
                                };

                                match serde_json::to_string_pretty(&cfg) {
                                    Ok(json) => {
                                        match File::create("key.json") {
                                            Ok(mut f) => {
                                                if f.write_all(json.as_bytes()).is_ok() {
                                                    // Success: signal main thread
                                                    let _ = self.startup_tx.send(key_val);

                                                    // ENABLE BROWSER
                                                    if let Ok(mut guard) = self.is_logged_in.write()
                                                    {
                                                        *guard = true;
                                                    }

                                                    let app = MyApp::new(
                                                        self.pool.clone(),
                                                        self.blacklist.clone(),
                                                        self.price.clone(),
                                                        self.total_token_count.clone(),
                                                        self.automata.clone(),
                                                        self.config.clone(),
                                                        self.trade_terminal.clone(),
                                                    );
                                                    next_state = Some(AppState::Running(app));
                                                } else {
                                                    *error_msg = Some(
                                                        "Failed to write to key.json".to_string(),
                                                    );
                                                }
                                            }
                                            Err(_) => {
                                                *error_msg =
                                                    Some("Failed to create key.json".to_string())
                                            }
                                        }
                                    }
                                    Err(_) => *error_msg = Some("Serialization error".to_string()),
                                }
                            }
                        }
                    });
                });
            }

            // --- MAIN APP ---
            AppState::Running(app) => {
                // Inside AppState::Running(app) branch of Launcher::update
                egui::TopBottomPanel::top("launcher_header").show(ctx, |ui| {
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        // ui.label("Wallet:");
                        // let addr = &app.account_data.wallet_public_key;
                        // let short_addr = format!("{}...{}", &addr[..6], &addr[addr.len()-6..]);
                        // ui.label(RichText::new(short_addr).color(Color32::LIGHT_BLUE).strong());

                        // // FIX IS HERE: ctx.copy_text
                        // if ui.button("📋").on_hover_text("Copy Wallet Address").clicked() {
                        //     ctx.copy_text(addr.clone());
                        // }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Logout").clicked() {
                                if let Ok(mut guard) = self.is_logged_in.write() {
                                    *guard = false;
                                }
                                next_state = Some(AppState::Login {
                                    input_key: String::new(),
                                    error_msg: None,
                                });
                            }
                        });
                    });
                    ui.add_space(5.0);
                });
                app.update(ctx, frame);
            }
        }

        if let Some(ns) = next_state {
            self.state = ns;
            ctx.request_repaint();
        }
    }
}
// ==============================================================================
// 3. MAIN APP IMPLEMENTATION
// ==============================================================================

pub struct MyApp {
    pub pool: Arc<Mutex<Pool>>,
    pub blacklist: Arc<Mutex<Blacklist>>,
    pub price: Arc<AtomicU64>,
    pub automata: Arc<Mutex<BuyAutomata>>,
    pub total_token_count: Arc<AtomicI64>,

    pub menu_open: bool,
    pub mcap_min: String,
    pub mcap_max: String,
    pub mcap_buy_max: String,
    pub mcap_buy_min: String,

    // token count fields
    pub token_count_min: String,
    pub token_count_max: String,
    pub token_count_buy_min: String,
    pub token_count_buy_max: String,

    // migration % fields (expects 0..100 values)
    pub migration_min: String,
    pub migration_max: String,
    pub migration_buy_min: String,
    pub migration_buy_max: String,

    pub filters: FilterSet,
    pub filters_buy: FilterSet,

    bribe_input: String,
    sol_input: String,
    slip_input: String,
    fee_input: String,

    // cached feed so ui can keep showing last known items if lock fails
    pub cached_feed: Vec<Token>,
    pub trade_terminal: Arc<RwLock<TradeTerminal>>,
}

impl MyApp {
    pub fn new(
        pool: Arc<Mutex<Pool>>,
        blacklist: Arc<Mutex<Blacklist>>,
        price: Arc<AtomicU64>,
        total: Arc<AtomicI64>,
        automata: Arc<Mutex<BuyAutomata>>,
        config: Option<AutoBuyConfig>,
        trade_terminal: Arc<RwLock<TradeTerminal>>,
    ) -> Self {
        // если конфиг есть, вытаскиваем значения, иначе пустые строки
        let (sol_input, fee_input, slip_input, bribe_input, filters_buy) =
            if let Some(cfg) = &config {
                (
                    (cfg.params.lamport_amount as f64 / 1_000_000_000.0).to_string(),
                    (cfg.params.priority_fee as f64 / 1_000_000_000.0).to_string(),
                    (cfg.params.slippage * 100.0).to_string(),
                    (cfg.params.bribe as f64 / 1_000_000_000.0).to_string(),
                    cfg.params.filters.clone(),
                )
            } else {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    FilterSet::new(),
                )
            };

        // грузим фильтры из файлов
        let filters = FilterSet::load("view_filters");
        let filters_buy = FilterSet::load("buy_view_filters");

        // утилита: переводит Filter -> (min, max) строки
        fn range_to_strings(f: Option<&Filters>) -> (String, String) {
            match f {
                Some(Filters::AverageDevMarketCap(r)) => (r.start.to_string(), r.end.to_string()),
                Some(Filters::TokenCount(r)) => (r.start.to_string(), r.end.to_string()),
                Some(Filters::MigrationPercentage(r)) => (r.start.to_string(), r.end.to_string()),
                None => (String::new(), String::new()),
            }
        }

        // достаём диапазоны из фильтров
        let (mcap_min, mcap_max) = range_to_strings(filters.filters.get(&Tag::AverageDevMarketCap));
        let (mcap_buy_min, mcap_buy_max) =
            range_to_strings(filters_buy.filters.get(&Tag::AverageDevMarketCap));

        let (token_count_min, token_count_max) =
            range_to_strings(filters.filters.get(&Tag::TokenCount));
        let (token_count_buy_min, token_count_buy_max) =
            range_to_strings(filters_buy.filters.get(&Tag::TokenCount));

        let (migration_min, migration_max) = match filters.filters.get(&Tag::MigrationPercentage) {
            Some(Filters::MigrationPercentage(r)) => ((r.start).to_string(), (r.end).to_string()),
            Some(_) => (String::new(), String::new()),
            None => (String::new(), String::new()),
        };

        let (migration_buy_min, migration_buy_max) = match filters_buy
            .filters
            .get(&Tag::MigrationPercentage)
        {
            Some(Filters::MigrationPercentage(r)) => ((r.start).to_string(), (r.end).to_string()),
            Some(_) => (String::new(), String::new()),
            None => (String::new(), String::new()),
        };

        Self {
            pool,
            automata,
            blacklist,
            price,
            total_token_count: total,
            menu_open: false,

            mcap_min,
            mcap_max,
            mcap_buy_min,
            mcap_buy_max,

            token_count_min,
            token_count_max,
            token_count_buy_min,
            token_count_buy_max,

            migration_min,
            migration_max,
            migration_buy_min,
            migration_buy_max,

            filters,
            filters_buy,

            sol_input,
            fee_input,
            slip_input,
            bribe_input,

            cached_feed: Vec::new(),
            trade_terminal,
            //account_data
        }
    }
}

use std::{fs, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeTerminal {
    Axiom,
    Padre,
}

impl TradeTerminal {
    pub fn save_to_file(&self, path: &str) -> io::Result<()> {
        let data = serde_json::to_string(self).unwrap();
        fs::write(path, data)
    }

    pub fn load_from_file(path: &str) -> io::Result<Self> {
        let data = fs::read_to_string(path)?;
        let terminal = serde_json::from_str(&data).unwrap();
        Ok(terminal)
    }

    pub fn url(&self, curve: &Pubkey) -> String {
        match self {
            TradeTerminal::Axiom => {
                format!("https://axiom.trade/meme/{}", curve)
            }
            TradeTerminal::Padre => {
                format!("https://trade.padre.gg/trade/solana/{}", curve)
            }
        }
    }
}

impl Drop for MyApp {
    fn drop(&mut self) {
        let _ = self.filters.to_file("view_filters");
        let _ = self.filters_buy.to_file("buy_view_filters");
    }
}

impl MyApp {
    fn open_token(&self, curve: &Pubkey) {
        let terminal = *self.trade_terminal.read().unwrap();
        let _ = open::that(terminal.url(&curve));
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.add_space(10.0);

            if ui.button("☰").clicked() {
                self.menu_open = !self.menu_open;
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.heading("Token Pool");
                let clear = ui.button("Clear");
                ui.separator();

                if clear.clicked() {
                    if let Ok(mut pool) = self.pool.try_lock() {
                        pool.clear();
                    }
                    // тоже очистим кэш чтобы не показывать старые данные
                    self.cached_feed.clear();
                }
            });

            ui.add_space(10.0);

            if self.menu_open {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("terminal:");
                        let mut current = *self.trade_terminal.read().unwrap();

                        if ui
                            .radio_value(&mut current, TradeTerminal::Axiom, "axiom")
                            .changed()
                            || ui
                                .radio_value(&mut current, TradeTerminal::Padre, "padre")
                                .changed()
                        {
                            *self.trade_terminal.write().unwrap() = current;
                            let _ = self
                                .trade_terminal
                                .read()
                                .unwrap()
                                .save_to_file("./terminal.json");
                        }
                    });

                    // --- average market cap ---
                    ui.label("median market cap range:");

                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("min:");
                        if ui.text_edit_singleline(&mut self.mcap_min).changed() {
                            changed = true;
                        }

                        ui.label("max:");
                        if ui.text_edit_singleline(&mut self.mcap_max).changed() {
                            changed = true;
                        }
                    });

                    if changed {
                        let min_mcap = self.mcap_min.parse::<u64>().unwrap_or(0);
                        let max_mcap = self.mcap_max.parse::<u64>().unwrap_or(100_000);

                        self.filters.add_filter(
                            Tag::AverageDevMarketCap,
                            Filters::AverageDevMarketCap(min_mcap..max_mcap),
                        );

                        if let Ok(mut pool) = self.pool.try_lock() {
                            pool.filters = self.filters.clone();
                        }
                    }

                    // --- token count range ---
                    ui.add_space(4.0);
                    ui.label("token count range:");
                    let mut changed_token = false;
                    ui.horizontal(|ui| {
                        ui.label("min:");
                        if ui.text_edit_singleline(&mut self.token_count_min).changed() {
                            changed_token = true;
                        }

                        ui.label("max:");
                        if ui.text_edit_singleline(&mut self.token_count_max).changed() {
                            changed_token = true;
                        }
                    });

                    if changed_token {
                        let min = self.token_count_min.parse::<u64>().unwrap_or(0);
                        let max = self.token_count_max.parse::<u64>().unwrap_or(100_000);

                        self.filters
                            .add_filter(Tag::TokenCount, Filters::TokenCount(min..max));

                        if let Ok(mut pool) = self.pool.try_lock() {
                            pool.filters = self.filters.clone();
                        }
                    }

                    // --- migration percentage range ---
                    ui.add_space(4.0);
                    ui.label("migration % range (0 - 100):");
                    let mut changed_mig = false;
                    ui.horizontal(|ui| {
                        ui.label("min:");
                        if ui.text_edit_singleline(&mut self.migration_min).changed() {
                            changed_mig = true;
                        }

                        ui.label("max:");
                        if ui.text_edit_singleline(&mut self.migration_max).changed() {
                            changed_mig = true;
                        }
                    });

                    if changed_mig {
                        let min = self.migration_min.parse::<u64>().unwrap_or(0);
                        let max = self.migration_max.parse::<u64>().unwrap_or(100);

                        self.filters.add_filter(
                            Tag::MigrationPercentage,
                            Filters::MigrationPercentage(min..max),
                        );

                        if let Ok(mut pool) = self.pool.try_lock() {
                            pool.filters = self.filters.clone();
                        }
                    }
                    if let Ok(mut automata) = self.automata.try_lock()
                        && automata.enabled
                    {
                        ui.separator();
                        ui.heading("auto-buy config");

                        ui.label("median market cap range:");

                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.label("min:");
                            if ui.text_edit_singleline(&mut self.mcap_buy_min).changed() {
                                changed = true;
                            }

                            ui.label("max:");
                            if ui.text_edit_singleline(&mut self.mcap_buy_max).changed() {
                                changed = true;
                            }
                        });

                        if changed {
                            let min_mcap = self.mcap_buy_min.parse::<u64>().unwrap_or(0);
                            let max_mcap = self.mcap_buy_max.parse::<u64>().unwrap_or(100_000);

                            self.filters_buy.add_filter(
                                Tag::AverageDevMarketCap,
                                Filters::AverageDevMarketCap(min_mcap..max_mcap),
                            );

                            automata.config.params.filters = self.filters_buy.clone();
                        }

                        // --- auto-buy token count ---
                        let mut changed_token_buy = false;
                        ui.add_space(4.0);
                        ui.label("token count range (auto-buy):");
                        ui.horizontal(|ui| {
                            ui.label("min:");
                            if ui
                                .text_edit_singleline(&mut self.token_count_buy_min)
                                .changed()
                            {
                                changed_token_buy = true;
                            }
                            ui.label("max:");
                            if ui
                                .text_edit_singleline(&mut self.token_count_buy_max)
                                .changed()
                            {
                                changed_token_buy = true;
                            }
                        });

                        if changed_token_buy {
                            let min = self.token_count_buy_min.parse::<u64>().unwrap_or(0);
                            let max = self.token_count_buy_max.parse::<u64>().unwrap_or(100_000);

                            self.filters_buy
                                .add_filter(Tag::TokenCount, Filters::TokenCount(min..max));

                            automata.config.params.filters = self.filters_buy.clone();
                        }

                        // --- auto-buy migration % ---
                        let mut changed_mig_buy = false;
                        ui.add_space(4.0);
                        ui.label("migration % range (auto-buy, 0 - 100):");
                        ui.horizontal(|ui| {
                            ui.label("min:");
                            if ui
                                .text_edit_singleline(&mut self.migration_buy_min)
                                .changed()
                            {
                                changed_mig_buy = true;
                            }
                            ui.label("max:");
                            if ui
                                .text_edit_singleline(&mut self.migration_buy_max)
                                .changed()
                            {
                                changed_mig_buy = true;
                            }
                        });

                        if changed_mig_buy {
                            let min = self.migration_buy_min.parse::<u64>().unwrap_or(0);
                            let max = self.migration_buy_max.parse::<u64>().unwrap_or(100);

                            self.filters_buy.add_filter(
                                Tag::MigrationPercentage,
                                Filters::MigrationPercentage(min..max),
                            );

                            automata.config.params.filters = self.filters_buy.clone();
                        }

                        let mut active = automata.active_twitter;
                        if ui.checkbox(&mut active, "enabled market cap").changed() {
                            automata.active_twitter = active;
                        }

                        let mut active = automata.active_migrate;
                        if ui.checkbox(&mut active, "enabled migrated").changed() {
                            automata.active_migrate = active;
                        }

                        let mut active = automata.active_whitelist;
                        if ui.checkbox(&mut active, "enabled whitelist").changed() {
                            automata.active_whitelist = active;
                            println!("{}", automata.active_whitelist);
                        }

                        // lamports
                        if ui.text_edit_singleline(&mut self.sol_input).changed() {
                            if let Ok(val) = self.sol_input.parse::<f64>() {
                                automata.config.params.lamport_amount =
                                    (val * 1_000_000_000.0) as u64;
                            } else {
                                automata.config.params.lamport_amount = 0
                            }
                        }
                        ui.label("amount (SOL)");

                        // priority fee
                        if ui.text_edit_singleline(&mut self.fee_input).changed() {
                            if let Ok(val) = self.fee_input.parse::<f64>() {
                                automata.config.params.priority_fee =
                                    (val * 1_000_000_000.0) as u64;
                            } else {
                                automata.config.params.priority_fee = 0
                            }
                        }
                        ui.label("priority fee (SOL)");

                        // slippage
                        if ui.text_edit_singleline(&mut self.slip_input).changed() {
                            if let Ok(val) = self.slip_input.parse::<f32>() {
                                automata.config.params.slippage = val / 100.0;
                            } else {
                                automata.config.params.slippage = 0.0
                            }
                        }
                        ui.label("slippage (0% - 100%)");

                        // bribe
                        if ui.text_edit_singleline(&mut self.bribe_input).changed() {
                            if let Ok(val) = self.bribe_input.parse::<f64>() {
                                automata.config.params.bribe = (val * 1_000_000_000.0) as u64;
                            } else {
                                automata.config.params.bribe = 1000
                            }
                        }
                        ui.label("bribe (0.000001 SOL min)");
                    }
                });
            }
        });

        // rest of your central panel: рендерим из кэша, и обновляем кэш, если лок успешен
        egui::CentralPanel::default().show(ctx, |ui| {
            // если удалось взять лок — обновляем cached_feed
            if let Ok(pool) = self.pool.try_lock() {
                // обновляем кэш (клонирование feed'а)
                self.cached_feed = pool.feed.clone();
            }

            ScrollArea::vertical().show(ui, |ui| {
                let fmt = human_format::Formatter::new();

                if self.cached_feed.is_empty() {
                    ui.label("no tokens yet");
                }

                for token in self.cached_feed.iter().rev() {
                    ui.vertical(|ui| {
                        ui.group(|ui| {
                            ui.set_min_width(180.0);

                            if ui
                                .link(
                                    RichText::new(format!("${}", token.ticker))
                                        .strong()
                                        .color(Color32::WHITE)
                                        .size(20.0),
                                )
                                .clicked()
                            {
                                self.open_token(&token.curve);
                            }

                            ui.label(RichText::new(&token.name).italics());
                        });

                        ui.vertical(|ui| {
                            if let Some(twitter) = token.twitter() {
                                ui.group(|ui| {
                                    ui.set_min_width(140.0);
                                    ui.heading("twitter");
                                    if ui
                                        .link(
                                            RichText::new(format!(
                                                "developer: @{}",
                                                twitter
                                                    .creator
                                                    .screen_name
                                                    .clone()
                                                    .unwrap_or("No screen name".to_owned())
                                            ))
                                            .color(Color32::LIGHT_BLUE),
                                        )
                                        .clicked()
                                    {
                                        let _ = open::that(format!(
                                            "https://twitter.com/intent/user?user_id={}",
                                            twitter.creator.id
                                        ));
                                    }

                                    if ui
                                        .link(
                                            RichText::new(format!("community: {}", twitter.name))
                                                .color(Color32::LIGHT_BLUE),
                                        )
                                        .clicked()
                                    {
                                        let _ = open::that(format!(
                                            "https://x.com/i/communities/{}",
                                            &twitter.id
                                        ));
                                    }
                                });
                            }
                        });

                        ui.vertical(|ui| {
                            ui.group(|ui| {
                                ui.set_min_width(160.0);
                                ui.heading("performance: ");

                                ui.label(
                                    RichText::new(format!("migration: "))
                                        .color(Color32::WHITE)
                                        .font(FontId::proportional(16.0)),
                                );

                                if let Some(migrated) = &token.migrated {
                                    let mut percent = ((migrated.counts.migrated_count as f32
                                                / migrated.counts.total_count as f32)
                                                * 100f32)
                                                .round();
                                    if percent.is_nan() {
                                        percent = 0f32;
                                    }
                                    
                                    ui.label(
                                        RichText::new(format!(
                                            "{}%",
                                            percent
                                        ))
                                        .color(Color32::LIGHT_GREEN)
                                        .font(FontId::proportional(16.0)),
                                    );
                                }

                                if let Some(performance) = &token.dev_performance {
                                    ui.label(
                                        RichText::new(format!(
                                            "coins created: {}",
                                            performance.count
                                        ))
                                        .color(Color32::LIGHT_YELLOW)
                                        .font(FontId::proportional(16.0)),
                                    );

                                    ui.label(
                                        RichText::new(format!(
                                            "median ath: {}$",
                                            fmt.format(performance.average_ath as f64)
                                        ))
                                        .color(Color32::YELLOW)
                                        .font(FontId::proportional(16.0)),
                                    );

                                    ui.add_space(10.0);
                                    ui.heading("last 3 coins");

                                    for (i, token) in performance.last_tokens.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            ui.label(format!("{} ", i + 1));
                                            ui.label(
                                                RichText::new("ath").color(egui::Color32::YELLOW),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "{}$",
                                                    fmt.format(token.ath as f64)
                                                ))
                                                .color(egui::Color32::YELLOW),
                                            );

                                            let name = if token.name.is_empty() {
                                                token.mint.to_string()
                                            } else {
                                                token.name.clone()
                                            };

                                            if ui.link(format!("{}", name)).clicked() {
                                                if let Ok(address) = Pubkey::from_str(&token.mint) {
                                                    let address = bounding_curve(&address).0;
                                                    self.open_token(&address);
                                                }
                                            }
                                        });
                                    }
                                } else {
                                    ui.label(RichText::new("no history found").italics());
                                }
                            });
                        });

                        ui.add_space(10.0);

                        if ui
                            .add(
                                egui::Button::new("Ban developer")
                                    .fill(egui::Color32::DARK_RED)
                                    .min_size(egui::vec2(120.0, 40.0)),
                            )
                            .clicked()
                        {
                            if let Ok(mut blacklist) = self.blacklist.try_lock() {
                                if let Some(twitter) = token.twitter() {
                                    blacklist.add(blacklist::Bannable::Twitter(
                                        twitter.creator.id.to_owned(),
                                    ));
                                }

                                blacklist.add(blacklist::Bannable::Wallet(token.dev));
                            }
                        }
                    });

                    ui.separator();
                    ui.add_space(5.0);
                }
            });
        });

        ctx.request_repaint();
    }
}

// ==============================================================================
// 4. HELPERS
// ==============================================================================

pub const PUMP_FUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

pub fn bounding_curve(mint: &Pubkey) -> (Pubkey, u8) {
    let seeds = &[b"bonding-curve", mint.as_ref()];
    Pubkey::find_program_address(seeds, &PUMP_FUN)
}
