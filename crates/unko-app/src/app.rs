//! Main window: library, player, menus, toolbar, settings.
//!
//! Layout follows Dolphin's desktop shell: menu bar, toolbar, central stack
//! (library or emulation surface), status bar, and one settings window with a
//! category rail. See `docs/frontend.md` for the mapping and the licensing
//! boundary (Dolphin is GPL; this is an original MIT implementation of the UX).
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Vec2};
use unko_core::{
    AudioFilterModel, AudioQuality, AudioSource, MaskEdges, Overscan, PortDevice, PpuResetBehavior,
    RamClear, Region, VideoFilter,
};

use crate::audio::AudioOutput;
use crate::cheats::{self, CheatEntry};
use crate::debugger::{self, Action};
use crate::game_props::GameProperties;
use crate::gamepad::{
    button_name, mapping_from_text, mapping_to_text, Gamepads, NesButton, PadMapping,
};
use crate::library::{self, GameEntry};
use crate::microphone::MicrophoneInput;
use crate::paths;
use crate::player::{self, Player};
use crate::playtime::{format_duration, PlayStats};
use crate::recording::RecordModes;
use crate::rom_manager::RomManager;
use crate::settings::{Aspect, HotkeyAction, PortInput, Settings, ShaderPreset, Theme};
use crate::slots::{default_slot_name, size_label, SLOT_COUNT};
use crate::vsconfig::VsConfig;
use unko_netplay::{
    add_port_mapping, discover, endpoint, external_ip, local_address_toward, stun_public_address,
    HolePunchTransport, MappingProtocol, MappingRequest, PredictionMode, RelayTransport,
    SessionConfig, UdpMesh,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Interface,
    Graphics,
    Audio,
    Controllers,
    Paths,
    Nes,
    Advanced,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::General,
        Tab::Interface,
        Tab::Graphics,
        Tab::Audio,
        Tab::Controllers,
        Tab::Paths,
        Tab::Nes,
        Tab::Advanced,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Interface => "Interface",
            Tab::Graphics => "Graphics",
            Tab::Audio => "Audio",
            Tab::Controllers => "Controllers",
            Tab::Paths => "Paths",
            Tab::Nes => "NES",
            Tab::Advanced => "Advanced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerAction {
    Resume,
    Pause,
    Stop,
    PlaySelected,
    OpenRom,
    Settings,
    RomManager,
    Rescan,
    ToggleView,
    Quit,
}

impl ControllerAction {
    const fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume game",
            Self::Pause => "Pause game",
            Self::Stop => "Stop game",
            Self::PlaySelected => "Play selected",
            Self::OpenRom => "Open ROM…",
            Self::Settings => "Settings…",
            Self::RomManager => "ROM Manager…",
            Self::Rescan => "Rescan library",
            Self::ToggleView => "Toggle grid/list",
            Self::Quit => "Quit Unko",
        }
    }
}

const PORT_DEVICES: [PortDevice; 12] = [
    PortDevice::Gamepad,
    PortDevice::Zapper,
    PortDevice::Paddle,
    PortDevice::Mouse,
    PortDevice::PowerPad,
    PortDevice::FamilyTrainer,
    PortDevice::FamilyKeyboard,
    PortDevice::SuborKeyboard,
    PortDevice::FourScore,
    PortDevice::Microphone,
    PortDevice::Tape,
    PortDevice::MiraclePiano,
];

fn port_device_label(device: PortDevice) -> &'static str {
    match device {
        PortDevice::Gamepad => "Gamepad",
        PortDevice::Zapper => "Zapper",
        PortDevice::Paddle => "Paddle",
        PortDevice::Mouse => "Mouse",
        PortDevice::PowerPad => "Power Pad",
        PortDevice::FamilyTrainer => "Family Trainer",
        PortDevice::FamilyKeyboard => "Family BASIC keyboard",
        PortDevice::SuborKeyboard => "Subor keyboard",
        PortDevice::FourScore => "Four Score",
        PortDevice::Microphone => "Microphone",
        PortDevice::Tape => "Data recorder",
        PortDevice::MiraclePiano => "Miracle Piano",
    }
}

fn port_text(override_port: Option<PortDevice>, global: PortDevice) -> String {
    override_port.map_or_else(
        || format!("Use global ({})", port_device_label(global)),
        |device| port_device_label(device).to_string(),
    )
}

fn region_name(region: Region) -> &'static str {
    match region {
        Region::Auto => "Auto",
        Region::Ntsc => "NTSC",
        Region::Pal => "PAL",
        Region::Dendy => "Dendy",
    }
}

fn region_text(override_region: Option<Region>, global: Region) -> String {
    override_region.map_or_else(
        || format!("Use global ({})", region_name(global)),
        |region| region_name(region).to_string(),
    )
}

fn controller_chord(pad: unko_core::InputState, previous: unko_core::InputState) -> bool {
    let both = |state: unko_core::InputState| {
        state.is_pressed(unko_core::InputState::START)
            && state.is_pressed(unko_core::InputState::SELECT)
    };
    both(pad) && !both(previous)
}

/// Application state.
pub struct App {
    settings: Settings,
    library: Vec<GameEntry>,
    search: String,
    selected: Option<usize>,
    player: Option<Player>,
    settings_open: bool,
    tab: Tab,
    about_open: bool,
    open_rom_open: bool,
    open_rom_text: String,
    fullscreen: bool,
    status: String,
    status_until: Option<Instant>,
    pending_play: Option<GameEntry>,
    window_title: String,
    gamepads: Option<Gamepads>,
    input_states: [unko_core::InputState; 4],
    gamepad_error: Option<String>,
    audio: Option<AudioOutput>,
    audio_error: Option<String>,
    hotkey_capture: Option<HotkeyAction>,
    active_mapping: PadMapping,
    paused_by_focus: bool,
    debug_on_start: bool,
    open_props_on_start: bool,
    devices_open: bool,
    cheats_open: bool,
    game_props_open: bool,
    game_props: GameProperties,
    game_props_entry: Option<GameEntry>,
    play_stats: PlayStats,
    slot_manager_open: bool,
    slot_clear_confirm: bool,
    slot_thumbnails: std::collections::HashMap<(String, u8, u64), egui::TextureHandle>,
    /// Library save stem of the running game (play-time key).
    play_stem: Option<String>,
    play_tick: Option<Instant>,
    play_pending: Duration,
    play_flush: Instant,
    cheats: Vec<CheatEntry>,
    cheat_code: String,
    cheat_addr: String,
    cheat_value: String,
    cheat_error: Option<String>,
    microphone: bool,
    piano_last: Option<u8>,
    piano_key_held: [bool; 128],
    vs_service: bool,
    vs_dips: u8,
    vs_primary: bool,
    microphone_input: Option<MicrophoneInput>,
    microphone_devices: Vec<String>,
    microphone_error: Option<String>,
    datach_code: String,
    confirm_reset: bool,
    rom_manager: RomManager,
    netplay_open: bool,
    netplay_host: bool,
    netplay_local_port: String,
    netplay_peer: String,
    netplay_status: String,
    netplay_delay: u32,
    netplay_prediction: PredictionMode,
    netplay_players: u8,
    netplay_slot: u8,
    netplay_public_endpoint: String,
    netplay_upnp: bool,
    netplay_upnp_status: String,
    netplay_stun_server: String,
    netplay_hole_punch: bool,
    netplay_relay_server: String,
    netplay_room: String,
    netplay_punch_deadline: Option<Instant>,
    netplay_pending_punch: Option<(HolePunchTransport, SessionConfig, u16)>,
    /// The game was running when the handshake started and should resume.
    netplay_resume_after_punch: bool,
    netplay_upnp_rx: Option<std::sync::mpsc::Receiver<Result<(String, String), String>>>,
    controller_last: unko_core::InputState,
    controller_repeat: Instant,
    controller_menu_open: bool,
    controller_menu_index: usize,
    controller_scroll: bool,
    settings_window_shown: bool,
    devices_window_shown: bool,
    cheats_window_shown: bool,
    rom_manager_window_shown: bool,
    debugger_window_shown: bool,
    window_generation: u32,
    #[cfg(not(target_os = "ios"))]
    gpu_shader: Option<crate::gpu_shader::Handle>,
}

/// Command-line overrides.
#[derive(Debug, Default)]
pub struct Args {
    /// ROM folder to scan.
    pub roms: Option<PathBuf>,
    /// ROM to launch immediately.
    pub rom: Option<PathBuf>,
    /// Open the settings window on launch.
    pub settings: bool,
    /// Open the debugger window when a game starts.
    pub debug: bool,
    /// Open the game properties window when a game starts.
    pub properties: bool,
    /// Open the NetPlay window on launch.
    pub netplay: bool,
    /// Open the save-state slot manager on launch.
    pub states: bool,
    /// Portable data folder override (`--data-dir`).
    pub data_dir: Option<PathBuf>,
    /// Use the folder beside the executable as the data root.
    pub portable: bool,
    /// Print the resolved data folder and exit.
    pub print_data_dir: bool,
    /// Open the ROM Manager window on launch.
    pub rom_manager: bool,
    /// Open the ROM Manager verification report after its initial scan.
    pub rom_verify: bool,
    /// DAT files to preload in the ROM Manager.
    pub dat: Vec<PathBuf>,
}

impl App {
    /// Builds the app, loading the library from the configured folder.
    pub fn new(cc: &eframe::CreationContext<'_>, mut settings: Settings, args: Args) -> Self {
        if let Some(dir) = args.roms {
            settings.rom_dir = paths::resolve(&dir);
        }
        let mut rom_manager = RomManager::new(settings.save_dir.clone(), settings.rom_dir.clone());
        rom_manager.preload(args.dat.clone(), args.rom_manager);
        rom_manager.set_verify_after_scan(args.rom_verify);
        if rom_manager.open && !args.dat.is_empty() {
            rom_manager.start_scan();
        }
        apply_theme(&cc.egui_ctx, settings.theme);
        let library = scan(&settings.rom_dir);
        let status = if library.is_empty() {
            format!(
                "No ROMs in {} — add .nes files or .zip/.7z/.rar archives",
                settings.rom_dir.display()
            )
        } else {
            format!("{} ROMs in {}", library.len(), settings.rom_dir.display())
        };
        let pending_play = args.rom.as_deref().and_then(resolve_first);
        let (gamepads, gamepad_error) = match Gamepads::new() {
            Ok(gamepads) => (Some(gamepads), None),
            Err(err) => (None, Some(err)),
        };
        let (audio, audio_error) =
            match AudioOutput::new(settings.sample_rate, settings.audio_latency_ms) {
                Ok(audio) => (Some(audio), None),
                Err(err) => (None, Some(err)),
            };
        if let Some(audio) = &audio {
            audio.set_volume(settings.volume);
            audio.set_muted(settings.muted);
        }
        let active_mapping = settings.pad_mapping;
        #[cfg(not(target_os = "ios"))]
        let gpu_shader = cc.gl.as_ref().and_then(|gl| {
            crate::gpu_shader::GpuShader::new(gl)
                .map(|shader| std::sync::Arc::new(std::sync::Mutex::new(shader)))
        });
        let play_stats = PlayStats::load(&settings.save_dir.join(PlayStats::FILE_NAME));
        let mut app = Self {
            settings,
            library,
            search: String::new(),
            selected: None,
            player: None,
            settings_open: false,
            tab: Tab::General,
            about_open: false,
            open_rom_open: false,
            open_rom_text: String::new(),
            fullscreen: false,
            status,
            status_until: None,
            pending_play,
            window_title: "Unko".into(),
            gamepads,
            input_states: [unko_core::InputState::empty(); 4],
            gamepad_error,
            audio,
            audio_error,
            hotkey_capture: None,
            active_mapping,
            paused_by_focus: false,
            debug_on_start: args.debug,
            open_props_on_start: args.properties,
            devices_open: false,
            cheats_open: false,
            game_props_open: false,
            game_props: GameProperties::default(),
            game_props_entry: None,
            play_stats,
            slot_manager_open: args.states,
            slot_clear_confirm: false,
            slot_thumbnails: std::collections::HashMap::new(),
            play_stem: None,
            play_tick: None,
            play_pending: Duration::ZERO,
            play_flush: Instant::now(),
            cheats: Vec::new(),
            cheat_code: String::new(),
            cheat_addr: String::new(),
            cheat_value: String::new(),
            cheat_error: None,
            microphone: false,
            piano_last: None,
            piano_key_held: [false; 128],
            vs_service: false,
            vs_dips: 0,
            vs_primary: true,
            microphone_input: None,
            microphone_devices: Vec::new(),
            microphone_error: None,
            datach_code: String::new(),
            confirm_reset: false,
            rom_manager,
            netplay_open: args.netplay,
            netplay_host: true,
            netplay_local_port: "7777".into(),
            netplay_peer: "127.0.0.1:7778".into(),
            netplay_status: String::new(),
            netplay_delay: 0,
            netplay_prediction: PredictionMode::LastInput,
            netplay_players: 2,
            netplay_slot: 1,
            netplay_public_endpoint: String::new(),
            netplay_upnp: false,
            netplay_upnp_status: String::new(),
            netplay_stun_server: "stun.l.google.com:19302".into(),
            netplay_hole_punch: false,
            netplay_relay_server: String::new(),
            netplay_room: "unko".into(),
            netplay_punch_deadline: None,
            netplay_pending_punch: None,
            netplay_resume_after_punch: false,
            netplay_upnp_rx: None,
            controller_last: unko_core::InputState::empty(),
            controller_repeat: Instant::now(),
            controller_menu_open: false,
            controller_menu_index: 0,
            controller_scroll: false,
            settings_window_shown: false,
            devices_window_shown: false,
            cheats_window_shown: false,
            rom_manager_window_shown: false,
            debugger_window_shown: false,
            window_generation: 0,
            #[cfg(not(target_os = "ios"))]
            gpu_shader,
        };
        app.status_until = Some(Instant::now() + Duration::from_secs(6));
        if app.settings.microphone_input {
            app.open_microphone();
        }
        if args.settings {
            app.settings_open = true;
        }
        if let Some(entry) = app.pending_play.take() {
            app.start_entry(entry);
        }
        if let Some(player) = &mut app.player {
            if args.debug {
                player.debug.open = true;
            }
        }
        app
    }

    fn handle_controller_ui(&mut self, ctx: &egui::Context, pad: unko_core::InputState) {
        let pressed = pad.bits() & !self.controller_last.bits();
        let now = Instant::now();
        let direction = pad.bits()
            & (unko_core::InputState::UP
                | unko_core::InputState::DOWN
                | unko_core::InputState::LEFT
                | unko_core::InputState::RIGHT);
        let repeat = direction != 0 && (pressed & direction != 0 || now >= self.controller_repeat);
        if self.controller_menu_open {
            let actions = self.controller_actions();
            if !actions.is_empty() {
                if repeat {
                    let down = pad.is_pressed(unko_core::InputState::DOWN)
                        || pad.is_pressed(unko_core::InputState::RIGHT);
                    let up = pad.is_pressed(unko_core::InputState::UP)
                        || pad.is_pressed(unko_core::InputState::LEFT);
                    if up {
                        self.controller_menu_index = self.controller_menu_index.saturating_sub(1);
                    } else if down {
                        self.controller_menu_index =
                            (self.controller_menu_index + 1).min(actions.len() - 1);
                    }
                    self.controller_repeat =
                        now + Duration::from_millis(if pressed != 0 { 340 } else { 110 });
                }
                if pressed & unko_core::InputState::A != 0 {
                    let action = actions[self.controller_menu_index.min(actions.len() - 1)];
                    self.controller_menu_open = false;
                    self.run_controller_action(ctx, action);
                } else if pressed & (unko_core::InputState::B | unko_core::InputState::START) != 0 {
                    self.controller_menu_open = false;
                }
            }
            self.controller_last = pad;
            return;
        }
        let blocked = self.settings_open
            || self.devices_open
            || self.cheats_open
            || self.about_open
            || self.open_rom_open
            || self.confirm_reset
            || self.rom_manager.open;
        if blocked {
            self.controller_last = pad;
            return;
        }
        let chord = controller_chord(pad, self.controller_last);
        let paused = self.player.as_ref().is_some_and(|player| player.paused);
        if chord {
            if let Some(player) = &mut self.player {
                if !player.paused {
                    player.toggle_pause();
                }
            }
            self.controller_menu_open = true;
            self.controller_menu_index = 0;
            self.controller_last = pad;
            return;
        }
        if self.player.is_none() {
            if repeat {
                self.move_library_selection(direction);
                self.controller_repeat =
                    now + Duration::from_millis(if pressed != 0 { 200 } else { 80 });
            }
            if pressed & unko_core::InputState::A != 0 {
                self.play_selected();
            }
            if pressed & unko_core::InputState::B != 0 {
                self.selected = None;
            }
            if pressed & unko_core::InputState::SELECT != 0 {
                self.settings.grid_view = !self.settings.grid_view;
                let _ = self.settings.save(&Settings::path());
            }
        } else if paused && pressed & unko_core::InputState::A != 0 {
            if let Some(player) = &mut self.player {
                player.toggle_pause();
            }
        }
        self.controller_last = pad;
    }

    fn move_library_selection(&mut self, direction: u16) {
        let indices = self.filtered();
        if indices.is_empty() {
            return;
        }
        let columns = if self.settings.grid_view {
            self.settings.grid_columns.clamp(1, 8) as usize
        } else {
            1
        };
        let position = self
            .selected
            .and_then(|selected| indices.iter().position(|index| *index == selected));
        let next = match (direction, position) {
            (unko_core::InputState::UP, Some(position)) => position.saturating_sub(columns),
            (unko_core::InputState::DOWN, Some(position)) => {
                (position + columns).min(indices.len() - 1)
            }
            (unko_core::InputState::LEFT, Some(position)) => position.saturating_sub(1),
            (unko_core::InputState::RIGHT, Some(position)) => (position + 1).min(indices.len() - 1),
            (unko_core::InputState::UP, None) => indices.len() - 1,
            (_, None) => 0,
            _ => return,
        };
        self.selected = Some(indices[next]);
        self.controller_scroll = true;
    }

    fn play_selected(&mut self) {
        let indices = self.filtered();
        if indices.is_empty() {
            return;
        }
        let index = self
            .selected
            .filter(|selected| indices.contains(selected))
            .unwrap_or(indices[0]);
        self.selected = Some(index);
        self.pending_play = Some(self.library[index].clone());
    }

    fn controller_actions(&self) -> Vec<ControllerAction> {
        let mut actions = Vec::new();
        if let Some(player) = &self.player {
            actions.push(if player.paused {
                ControllerAction::Resume
            } else {
                ControllerAction::Pause
            });
            actions.push(ControllerAction::Stop);
        } else {
            actions.push(ControllerAction::PlaySelected);
        }
        actions.extend([
            ControllerAction::OpenRom,
            ControllerAction::Settings,
            ControllerAction::RomManager,
            ControllerAction::Rescan,
            ControllerAction::ToggleView,
            ControllerAction::Quit,
        ]);
        actions
    }

    fn run_controller_action(&mut self, ctx: &egui::Context, action: ControllerAction) {
        match action {
            ControllerAction::Resume => {
                if let Some(player) = &mut self.player {
                    if player.paused {
                        player.toggle_pause();
                    }
                }
            }
            ControllerAction::Pause => {
                if let Some(player) = &mut self.player {
                    player.toggle_pause();
                }
            }
            ControllerAction::Stop => {
                self.stop_game();
            }
            ControllerAction::PlaySelected => {
                self.play_selected();
            }
            ControllerAction::OpenRom => {
                self.open_rom_open = true;
            }
            ControllerAction::Settings => {
                self.settings_open = true;
                self.tab = Tab::General;
            }
            ControllerAction::RomManager => {
                self.rom_manager.request_focus();
            }
            ControllerAction::Rescan => {
                self.scan();
            }
            ControllerAction::ToggleView => {
                self.settings.grid_view = !self.settings.grid_view;
                let _ = self.settings.save(&Settings::path());
            }
            ControllerAction::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn controller_menu(&mut self, ctx: &egui::Context) {
        if !self.controller_menu_open {
            return;
        }
        let actions = self.controller_actions();
        let mut chosen = None;
        egui::Window::new("Controller menu")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .show(ctx, |ui| {
                ui.heading("Menu");
                for (index, action) in actions.iter().enumerate() {
                    if ui
                        .selectable_label(self.controller_menu_index == index, action.label())
                        .clicked()
                    {
                        chosen = Some(*action);
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new("D-pad move · A select · B close")
                        .weak()
                        .small(),
                );
            });
        if let Some(action) = chosen {
            self.controller_menu_open = false;
            self.run_controller_action(ctx, action);
        }
    }

    fn scan(&mut self) {
        let mut roots = vec![self.settings.rom_dir.clone()];
        for root in &self.settings.library_roots {
            if root.system == "nes" {
                roots.push(root.path.clone());
            }
        }
        let mut library = Vec::new();
        for root in roots {
            library.extend(scan(&root));
        }
        let mut seen = std::collections::HashSet::new();
        library.retain(|entry| seen.insert(entry.source.display_path()));
        self.library = library;
        self.selected = None;
        self.status = if self.library.is_empty() {
            format!(
                "No ROMs in {} — add .nes files or .zip/.7z/.rar archives",
                self.settings.rom_dir.display()
            )
        } else if self.settings.library_roots.is_empty() {
            format!(
                "{} ROMs in {}",
                self.library.len(),
                self.settings.rom_dir.display()
            )
        } else {
            format!(
                "{} ROMs in {} + {} extra root(s)",
                self.library.len(),
                self.settings.rom_dir.display(),
                self.settings.library_roots.len()
            )
        };
        self.status_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_until = Some(Instant::now() + Duration::from_secs(6));
    }

    fn handle_drop(&mut self, files: Vec<egui::DroppedFile>) {
        let mut directories = Vec::new();
        let mut first_file = None;
        for file in files {
            let Some(path) = file.path else {
                continue;
            };
            if path.is_dir() {
                directories.push(path);
            } else if path.is_file() {
                if let Some(parent) = path.parent() {
                    directories.push(parent.to_path_buf());
                }
                first_file.get_or_insert(path);
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut added = false;
        for directory in directories {
            if !seen.insert(directory.clone()) {
                continue;
            }
            if directory == self.settings.rom_dir
                || self
                    .settings
                    .library_roots
                    .iter()
                    .any(|root| root.system == "nes" && root.path == directory)
            {
                continue;
            }
            self.settings
                .library_roots
                .push(crate::settings::LibraryRoot {
                    system: "nes".into(),
                    path: directory,
                });
            added = true;
        }
        if added {
            self.scan();
        }
        if let Some(path) = first_file {
            self.start_game(&path);
        } else if added {
            self.set_status("Added dropped folder to the library");
        } else {
            self.set_status("Drop a ROM, archive or folder to open it");
        }
    }

    fn start_game(&mut self, path: &Path) {
        match library::probe_path(path) {
            Ok(entries) => match entries.into_iter().next() {
                Some(entry) => self.start_entry(entry),
                None => self.set_status(format!("No playable NES image in {}", path.display())),
            },
            Err(err) => self.set_status(format!("{err}")),
        }
    }

    fn start_entry(&mut self, entry: GameEntry) {
        self.persist_vs_config();
        let mut settings = self.settings.clone();
        if let Some(audio) = &self.audio {
            settings.sample_rate = audio.sample_rate();
        }
        let game_props = GameProperties::load(&self.game_props_path(&entry));
        match Player::start_with(&entry, &settings, game_props) {
            Ok(player) => {
                self.flush_play_stats();
                self.play_pending = Duration::ZERO;
                self.play_tick = Some(Instant::now());
                self.play_stem = Some(entry.save_stem());
                self.play_stats.begin_session(&entry.save_stem());
                self.flush_play_stats();
                self.game_props = game_props;
                self.game_props_entry = Some(entry.clone());
                self.active_mapping = if self.settings.per_game_input {
                    let path = self
                        .settings
                        .save_dir
                        .join(format!("{}.pad", entry.save_stem()));
                    fs::read_to_string(path)
                        .map(|text| mapping_from_text(&text))
                        .unwrap_or(self.settings.pad_mapping)
                } else {
                    self.settings.pad_mapping
                };
                self.set_status(format!("Loaded {} — {}", entry.title, player.core_label()));
                if self.open_props_on_start {
                    self.game_props_open = true;
                    self.open_props_on_start = false;
                }
                self.player = Some(player);
                let vs = self
                    .player
                    .as_ref()
                    .filter(|player| player.is_vs_system())
                    .map_or_else(VsConfig::default, |player| {
                        VsConfig::load(&VsConfig::path(&self.settings.save_dir, &player.stem))
                    });
                self.vs_service = false;
                self.vs_dips = vs.dips;
                self.vs_primary = vs.primary;
                if let Some(player) = &mut self.player {
                    let path = self
                        .settings
                        .save_dir
                        .join(format!("{}.cheats", player.stem));
                    player.set_vs_input(self.vs_service, self.vs_dips, self.vs_primary);
                    self.cheats = cheats::load(&path);
                }
                self.apply_cheats();
                if self.debug_on_start {
                    if let Some(player) = &mut self.player {
                        player.debug.open = true;
                    }
                }
            }
            Err(err) => self.set_status(format!("Load failed: {err}")),
        }
    }

    fn start_recording(&mut self, modes: RecordModes) {
        let Some(player) = &mut self.player else {
            return;
        };
        match player.start_recording(modes) {
            Ok(paths) => self.set_status(format!("Recording to {paths}")),
            Err(err) => self.set_status(format!("Recording failed: {err}")),
        }
    }

    fn stop_game(&mut self) {
        self.flush_play_stats();
        self.persist_vs_config();
        self.play_stem = None;
        self.paused_by_focus = false;
        if self.settings.per_game_input {
            if let Some(player) = &self.player {
                let path = self.settings.save_dir.join(format!("{}.pad", player.stem));
                let _ = fs::write(path, mapping_to_text(self.active_mapping));
            }
        }
        if let Some(mut player) = self.player.take() {
            player.shutdown();
            self.set_status(format!("Stopped {}", player.title));
        }
        self.cheats.clear();
        self.cheats_open = false;
        self.cheat_error = None;
    }

    /// Writes the current Vs. DIP/primary choices for the running game.
    fn persist_vs_config(&self) {
        let Some(player) = &self.player else {
            return;
        };
        if !player.is_vs_system() {
            return;
        }
        let config = VsConfig {
            dips: self.vs_dips,
            primary: self.vs_primary,
        };
        let _ = config.save(&VsConfig::path(&self.settings.save_dir, &player.stem));
    }

    fn apply_cheats(&mut self) {
        let active: Vec<unko_core::Cheat> = self
            .cheats
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.cheat.clone())
            .collect();
        if let Some(player) = &mut self.player {
            player.set_cheats(&active);
        }
    }

    fn save_cheats(&mut self) {
        let Some(player) = &self.player else {
            return;
        };
        let path = self
            .settings
            .save_dir
            .join(format!("{}.cheats", player.stem));
        if let Err(err) = cheats::save(&path, &self.cheats) {
            self.set_status(format!("Cheat save failed: {err}"));
        }
    }

    fn open_microphone(&mut self) {
        self.microphone_devices = MicrophoneInput::devices();
        let device = self.settings.microphone_device.clone();
        let device = if device.is_empty() {
            None
        } else {
            Some(device.as_str())
        };
        match MicrophoneInput::new(device, self.settings.microphone_threshold) {
            Ok(input) => {
                self.microphone_input = Some(input);
                self.microphone_error = None;
            }
            Err(err) => {
                self.microphone_input = None;
                self.microphone_error = Some(err);
            }
        }
    }

    fn close_microphone(&mut self) {
        self.microphone_input = None;
        self.microphone_error = None;
    }

    fn apply_settings(&mut self, ctx: &egui::Context, old: &Settings) {
        if old.save_dir != self.settings.save_dir {
            self.play_stats = PlayStats::load(&self.play_stats_path());
        }
        self.settings.zapper_port2 = self.settings.port2_device == PortDevice::Zapper;
        if let Some(player) = &mut self.player {
            player.set_filter(self.settings.filter);
            player.set_palette(self.settings.palette);
            if self.settings.palette == unko_core::Palette::Custom {
                if let Some(table) = crate::settings::load_palette(&self.settings.custom_palette) {
                    player.set_palette_table(table);
                }
            }
            let rate = self
                .audio
                .as_ref()
                .map_or(self.settings.sample_rate, |audio| audio.sample_rate());
            player.set_sample_rate(rate);
        }
        if self.settings.pad_mapping != old.pad_mapping {
            self.active_mapping = self.settings.pad_mapping;
        }
        if let Some(player) = &mut self.player {
            if self.settings.audio_quality != old.audio_quality {
                player.set_audio_quality(self.settings.audio_quality);
            }
            if self.settings.audio_filter_model() != old.audio_filter_model() {
                player.set_audio_filters(self.settings.audio_filter_model());
            }
            if self.settings.effective_channel_gains() != old.effective_channel_gains() {
                player.set_channel_gains(self.settings.effective_channel_gains());
            }
            if self.settings.supersample != old.supersample {
                player.set_supersample(self.settings.supersample);
            }
            if self.settings.speed_percent != old.speed_percent {
                player.set_speed(self.settings.speed_percent);
            }
            if self.settings.rewind_mb != old.rewind_mb {
                player.set_rewind_mb(self.settings.rewind_mb);
            }
            if self.settings.run_ahead != old.run_ahead {
                player.set_run_ahead(self.settings.run_ahead);
            }
            if self.settings.audio_source != old.audio_source {
                player.set_audio_source(self.settings.audio_source);
            }
            if self.settings.ram_clear != old.ram_clear {
                player.set_ram_clear(self.settings.ram_clear);
            }
            if self.settings.ppu_reset != old.ppu_reset {
                player.set_ppu_reset_behavior(self.settings.ppu_reset);
            }
            if self.settings.mmc3_alternate != old.mmc3_alternate {
                player.set_mmc3_alternate(self.settings.mmc3_alternate);
            }
            if self.settings.famicom_3d != old.famicom_3d {
                player.set_famicom_3d(self.settings.famicom_3d);
            }
            if self.settings.fds_speed != old.fds_speed {
                player.set_fds_speed(self.settings.fds_speed);
            }
            if self.settings.fds_auto_side != old.fds_auto_side {
                player.set_fds_auto_side_swap(self.settings.fds_auto_side);
            }
        }
        if let Some(audio) = &self.audio {
            audio.set_volume(self.settings.volume);
            audio.set_muted(self.settings.muted);
            if self.settings.audio_latency_ms != old.audio_latency_ms {
                audio.set_latency_ms(self.settings.audio_latency_ms);
            }
        }
        let microphone_changed = self.settings.microphone_input != old.microphone_input
            || self.settings.microphone_device != old.microphone_device
            || self.settings.microphone_threshold != old.microphone_threshold;
        if microphone_changed {
            if self.settings.microphone_input {
                self.open_microphone();
            } else {
                self.close_microphone();
            }
        }
        apply_theme(ctx, self.settings.theme);
        if self.settings.rom_dir != old.rom_dir {
            self.scan();
        }
        let _ = self.settings.save(&Settings::path());
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.hotkey_capture {
            let key = ctx.input(|i| {
                i.events.iter().find_map(|event| {
                    if let egui::Event::Key {
                        key, pressed: true, ..
                    } = event
                    {
                        Some(*key)
                    } else {
                        None
                    }
                })
            });
            if let Some(key) = key {
                self.settings.hotkeys.set(action, key);
                self.hotkey_capture = None;
                let _ = self.settings.save(&Settings::path());
                self.set_status(format!("{} → {}", action.label(), key.name()));
            }
            return;
        }
        let (step_instruction, step_frame) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::F10),
                input.key_pressed(egui::Key::F9),
            )
        });
        if let Some(player) = &mut self.player {
            if step_instruction {
                player.apply_debug(Action::StepInstruction, ctx);
            }
            if step_frame {
                player.apply_debug(Action::StepFrame, ctx);
            }
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let keys = self.settings.hotkeys;
        let (fast_forward, frame_advance) = ctx.input(|i| {
            (
                i.key_down(keys.fast_forward),
                i.key_pressed(keys.frame_advance),
            )
        });
        if let Some(player) = &mut self.player {
            player.set_fast_forward(fast_forward);
            if frame_advance {
                player.paused = true;
                player.advance_frame(ctx);
            }
        }
        let piano_keyboard_active = self.devices_open
            && self.settings.piano_keyboard
            && (self.settings.port1_device == PortDevice::MiraclePiano
                || self.settings.port2_device == PortDevice::MiraclePiano);
        let (save, load, fullscreen, pause, screenshot, reset) = ctx.input(|i| {
            (
                i.key_pressed(keys.save),
                i.key_pressed(keys.load),
                i.key_pressed(keys.fullscreen),
                i.key_pressed(keys.pause) && !piano_keyboard_active,
                i.key_pressed(keys.screenshot),
                i.key_pressed(keys.reset),
            )
        });
        if fullscreen {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if let Some(player) = &mut self.player {
            if save {
                player.save_state(player.slot);
            }
            if load {
                player.load_state(player.slot);
            }
            if pause {
                player.toggle_pause();
            }
            if screenshot {
                player.screenshot();
            }
            if reset {
                player.reset();
            }
        }
    }

    fn menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu-bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        self.open_rom_open = true;
                        ui.close_menu();
                    }
                    if ui.button("Set ROM folder...").clicked() {
                        self.tab = Tab::Paths;
                        self.settings_open = true;
                        ui.close_menu();
                    }
                    if ui.button("Rescan library").clicked() {
                        self.scan();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Emulation", |ui| {
                    let has_game = self.player.is_some();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Play / Pause"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.toggle_pause();
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Stop"))
                        .clicked()
                    {
                        self.stop_game();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Reset"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.reset();
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Frame advance (.)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.paused = true;
                            player.advance_frame(ctx);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Rewind (Backspace)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.paused = true;
                            player.rewind_step(ctx);
                        }
                        ui.close_menu();
                    }
                    ui.menu_button("Rewind buffer", |ui| {
                        for budget in [64, 128, 256] {
                            if ui
                                .radio_value(
                                    &mut self.settings.rewind_mb,
                                    budget,
                                    format!("{budget} MiB"),
                                )
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_rewind_mb(budget);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                    });
                    ui.menu_button("Run-ahead", |ui| {
                        for frames in 0..=4 {
                            let label = if frames == 0 {
                                "Off".to_string()
                            } else {
                                format!("{frames} frame{}", if frames == 1 { "" } else { "s" })
                            };
                            if ui
                                .radio_value(&mut self.settings.run_ahead, frames, label)
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_run_ahead(frames);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(
                                "Runs extra frames to hide input latency; suspended at \
                                 non-100% speeds, during movies and while debugging.",
                            )
                            .weak()
                            .small(),
                        );
                    });
                    let disk_sides = self
                        .player
                        .as_ref()
                        .map_or(0, crate::player::Player::disk_sides);
                    if disk_sides > 0 {
                        let current = self
                            .player
                            .as_ref()
                            .map_or(0, crate::player::Player::current_disk_side);
                        ui.menu_button("Disk side", |ui| {
                            for side in 0..disk_sides {
                                if ui
                                    .radio(current == side, format!("Side {}", side + 1))
                                    .clicked()
                                {
                                    if let Some(player) = &mut self.player {
                                        player.insert_disk_side(Some(side));
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                    ui.menu_button("Movie", |ui| {
                        let has_game = self.player.is_some();
                        let recording = self
                            .player
                            .as_ref()
                            .is_some_and(|player| player.movie_recording());
                        let playing = self
                            .player
                            .as_ref()
                            .is_some_and(|player| player.movie_progress().is_some());
                        if ui
                            .add_enabled(
                                has_game && !recording && !playing,
                                egui::Button::new("Record movie"),
                            )
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                match player.start_movie_recording() {
                                    Ok(()) => self.set_status("Recording input movie"),
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(recording, egui::Button::new("Stop recording"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                let path = player
                                    .save_dir
                                    .join("recordings")
                                    .join(format!("{}.movie", player.stem));
                                match player.stop_movie_recording(&path) {
                                    Ok(()) => {
                                        self.set_status(format!(
                                            "Saved movie to {}",
                                            path.display()
                                        ));
                                    }
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        #[cfg(not(any(target_os = "android", target_os = "ios")))]
                        if ui
                            .add_enabled(
                                has_game && !recording && !playing,
                                egui::Button::new("Play movie..."),
                            )
                            .clicked()
                        {
                            let start = self
                                .player
                                .as_ref()
                                .map(|player| player.save_dir.join("recordings"));
                            let mut dialog = rfd::FileDialog::new()
                                .add_filter("Unko input movie", &["movie"])
                                .set_title("Play input movie");
                            if let Some(directory) = start {
                                dialog = dialog.set_directory(directory);
                            }
                            if let Some(path) = dialog.pick_file() {
                                match crate::movie::Movie::load(&path) {
                                    Ok(movie) => {
                                        if let Some(player) = &mut self.player {
                                            match player.start_movie_playback(movie) {
                                                Ok(()) => self.set_status(format!(
                                                    "Playing {}",
                                                    path.display()
                                                )),
                                                Err(err) => {
                                                    self.set_status(format!("Movie: {err}"));
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(playing, egui::Button::new("Stop playback"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                player.stop_movie_playback();
                            }
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Speed", |ui| {
                        for percent in [25, 50, 100, 150, 200, 300] {
                            if ui
                                .radio_value(
                                    &mut self.settings.speed_percent,
                                    percent,
                                    format!("{percent}%"),
                                )
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_speed(percent);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                        ui.separator();
                        ui.label(RichText::new("Hold Tab to fast-forward").weak().small());
                    });
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Save state (F5)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            let slot = player.slot;
                            player.save_state(slot);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Load state (F8)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            let slot = player.slot;
                            player.load_state(slot);
                        }
                        ui.close_menu();
                    }
                    let thumbs: Vec<Option<Vec<u8>>> = (0..10u8)
                        .map(|slot| self.player.as_ref().and_then(|p| p.thumbnail(slot)))
                        .collect();
                    let thumb_stem = self
                        .player
                        .as_ref()
                        .map(|p| p.stem.clone())
                        .unwrap_or_default();
                    ui.menu_button("State slot", |ui| {
                        for slot in 0..10u8 {
                            ui.horizontal(|ui| {
                                if let Some(bytes) = &thumbs[slot as usize] {
                                    if bytes.len() == 64 * 60 * 4 {
                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                            [64, 60],
                                            bytes,
                                        );
                                        let texture = ui.ctx().load_texture(
                                            format!("{thumb_stem}-thumb-{slot}"),
                                            image,
                                            egui::TextureOptions::NEAREST,
                                        );
                                        ui.image((texture.id(), egui::vec2(48.0, 45.0)));
                                    }
                                }
                                let selected = self.player.as_ref().is_some_and(|p| p.slot == slot);
                                if ui
                                    .add_enabled(
                                        has_game,
                                        egui::Button::new(format!("Slot {slot}"))
                                            .selected(selected),
                                    )
                                    .clicked()
                                {
                                    if let Some(player) = &mut self.player {
                                        player.slot = slot;
                                    }
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                    if ui
                        .add_enabled(has_game, egui::Button::new("Save states..."))
                        .clicked()
                    {
                        self.slot_manager_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Debugger..."))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.debug.open = true;
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Step instruction (F10)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.apply_debug(Action::StepInstruction, ctx);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Step frame (F9)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.apply_debug(Action::StepFrame, ctx);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Screenshot"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.screenshot();
                        }
                        ui.close_menu();
                    }
                    if ui.button("Fullscreen (F11)").clicked() {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .selectable_label(!self.settings.grid_view, "List")
                        .clicked()
                    {
                        self.settings.grid_view = false;
                        ui.close_menu();
                    }
                    if ui
                        .selectable_label(self.settings.grid_view, "Grid")
                        .clicked()
                    {
                        self.settings.grid_view = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .checkbox(&mut self.settings.show_status_bar, "Status bar")
                        .changed()
                    {
                        let _ = self.settings.save(&Settings::path());
                    }
                    if ui.button("Fullscreen (F11)").clicked() {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Tools", |ui| {
                    for tab in [
                        Tab::Graphics,
                        Tab::Audio,
                        Tab::Controllers,
                        Tab::Nes,
                        Tab::Paths,
                    ] {
                        if ui.button(tab.label()).clicked() {
                            self.tab = tab;
                            self.settings_open = true;
                            ui.close_menu();
                        }
                    }
                    if ui.button("Devices").clicked() {
                        self.devices_open = true;
                        ui.close_menu();
                    }
                    if ui.button("ROM Manager…").clicked() {
                        self.rom_manager.request_focus();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.player.is_some(), egui::Button::new("NetPlay…"))
                        .clicked()
                    {
                        self.netplay_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.player.is_some(), egui::Button::new("Cheats"))
                        .clicked()
                    {
                        self.cheats_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.game_props_entry.is_some(),
                            egui::Button::new("Game properties…"),
                        )
                        .clicked()
                    {
                        self.game_props_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Recording", |ui| {
                        let has_game = self.player.is_some();
                        let recording = self.player.as_ref().is_some_and(|p| p.recording());
                        if ui
                            .add_enabled(has_game && !recording, egui::Button::new("Record WAV"))
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: true,
                                png: false,
                                gif: false,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                has_game && !recording,
                                egui::Button::new("Record PNG frames"),
                            )
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: false,
                                png: true,
                                gif: false,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(has_game && !recording, egui::Button::new("Record GIF"))
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: false,
                                png: false,
                                gif: true,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                has_game && !recording,
                                egui::Button::new("Record WAV + PNG + GIF"),
                            )
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: true,
                                png: true,
                                gif: true,
                            });
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(recording, egui::Button::new("Stop recording"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                let frames = player.recording_frames();
                                player.stop_recording();
                                self.set_status(format!("Recording stopped after {frames} frames"));
                            }
                            ui.close_menu();
                        }
                    });
                    ui.separator();
                    if ui.button("Settings...").clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Unko").clicked() {
                        self.about_open = true;
                        ui.close_menu();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Unko").weak());
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(38.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button("Open").clicked() {
                        self.open_rom_open = true;
                    }
                    if ui.button("Refresh").clicked() {
                        self.scan();
                    }
                    ui.separator();
                    let has_game = self.player.is_some();
                    let paused = self.player.as_ref().is_some_and(|p| p.paused);
                    if ui
                        .add_enabled(
                            has_game,
                            egui::Button::new(if paused { "Resume" } else { "Pause" }),
                        )
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.toggle_pause();
                        }
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Stop"))
                        .clicked()
                    {
                        self.stop_game();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Reset"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.reset();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Fullscreen"))
                        .clicked()
                    {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Screenshot"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.screenshot();
                        }
                    }
                    ui.separator();
                    if ui.button("Config").clicked() {
                        self.settings_open = true;
                    }
                    if ui.button("Graphics").clicked() {
                        self.tab = Tab::Graphics;
                        self.settings_open = true;
                    }
                    if ui.button("Controllers").clicked() {
                        self.tab = Tab::Controllers;
                        self.settings_open = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(player) = &self.player {
                            ui.label(RichText::new(&player.title).strong());
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status-bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                match &self.player {
                    Some(player) => {
                        let state = if player.paused { "Paused" } else { "Running" };
                        ui.label(RichText::new(state).strong());
                        ui.separator();
                        ui.label(&player.title);
                        ui.separator();
                        let (width, height) = player.resolution();
                        ui.label(format!("{width}×{height}"));
                        ui.separator();
                        ui.label(format!("{:.1} fps", player.fps));
                        ui.separator();
                        let speed = if player.fast_forward() {
                            "FF".to_string()
                        } else {
                            format!("{}%", player.speed_percent())
                        };
                        ui.label(speed);
                        ui.separator();
                        if player.rewind_available() > 0 {
                            ui.label(format!("rewind {}", player.rewind_available()));
                            ui.separator();
                        }
                        ui.label(format!("slot {}", player.slot));
                        ui.separator();
                        let sides = player.disk_sides();
                        if sides > 0 {
                            ui.label(format!(
                                "FDS side {}/{}",
                                player.current_disk_side() + 1,
                                sides
                            ));
                            ui.separator();
                        }
                        ui.label(format!("{:.0} Hz", player.sample_rate()));
                        if player.recording() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("REC {}", player.recording_frames()))
                                    .color(Color32::from_rgb(0xE0, 0x60, 0x60)),
                            );
                        }
                        if player.movie_recording() {
                            ui.separator();
                            let frames = player.movie_recorded_frames();
                            ui.label(
                                RichText::new(format!("MOVIE REC {frames}"))
                                    .color(Color32::from_rgb(0xE0, 0x60, 0x60)),
                            );
                        }
                        if let Some((index, total)) = player.movie_progress() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("MOVIE {index}/{total}"))
                                    .color(Color32::from_rgb(0x60, 0xC0, 0xE0)),
                            );
                        }
                    }
                    None => {
                        ui.label("No game loaded");
                        ui.separator();
                        ui.label(format!("{} ROMs", self.library.len()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.status.is_empty()
                        && self.status_until.is_none_or(|until| Instant::now() < until)
                    {
                        ui.label(RichText::new(&self.status).weak());
                    }
                });
            });
        });
    }

    fn library(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            let before = (self.settings.grid_columns, self.settings.list_columns);
            ui.horizontal(|ui| {
                ui.heading("Library");
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search games...")
                        .desired_width(240.0),
                );
                if !self.search.is_empty() && ui.button("Clear").clicked() {
                    self.search.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.settings.grid_view {
                        "List view"
                    } else {
                        "Grid view"
                    };
                    if ui.button(label).clicked() {
                        self.settings.grid_view = !self.settings.grid_view;
                        let _ = self.settings.save(&Settings::path());
                    }
                    ui.menu_button("Columns", |ui| {
                        ui.label("Grid columns (1-8)");
                        ui.add(egui::Slider::new(&mut self.settings.grid_columns, 1..=8));
                        ui.separator();
                        ui.label("List columns");
                        let mut mask = self.settings.list_columns;
                        for (name, bit) in LIST_COLUMN_BITS {
                            let mut shown = mask & bit != 0;
                            if ui.checkbox(&mut shown, name).changed() {
                                if shown {
                                    mask |= bit;
                                } else {
                                    mask &= !bit;
                                }
                            }
                        }
                        self.settings.list_columns = mask;
                    });
                    if ui.button("Rescan").clicked() {
                        self.scan();
                    }
                });
            });
            if before != (self.settings.grid_columns, self.settings.list_columns) {
                let _ = self.settings.save(&Settings::path());
            }
            ui.separator();

            let indices = self.filtered();
            if indices.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(
                        RichText::new("No games found")
                            .size(18.0)
                            .color(Color32::GRAY),
                    );
                    ui.add_space(8.0);
                    ui.label(format!(
                        "Put .nes files in {} or change Paths in Settings.",
                        self.settings.rom_dir.display()
                    ));
                });
                self.controller_scroll = false;
                return;
            }

            if self.settings.grid_view {
                self.grid(ui, &indices);
            } else {
                self.table(ui, &indices);
            }
            self.controller_scroll = false;
        });
    }

    fn filtered(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.library
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.title.to_lowercase().contains(&query)
                    || entry.file_name.to_lowercase().contains(&query)
                    || entry.mapper_label().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn table(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        const COLUMNS: [(&str, u8, f32); 7] = [
            ("System", 0x01, 0.9),
            ("File", 0x02, 2.0),
            ("Size", 0x04, 0.9),
            ("Region", 0x08, 0.9),
            ("Mapper", 0x10, 1.0),
            ("Modified", 0x20, 1.4),
            ("Played", 0x40, 0.9),
        ];
        let width = ui.available_width();
        let title_width = (width * 0.28).clamp(180.0, 360.0);
        let visible: Vec<(&str, u8, f32)> = COLUMNS
            .iter()
            .copied()
            .filter(|(_, bit, _)| self.settings.list_columns & bit != 0)
            .collect();
        let total_weight: f32 = visible.iter().map(|(_, _, weight)| weight).sum();
        let usable = (width - title_width - 28.0).max(120.0);
        let widths: Vec<f32> = visible
            .iter()
            .map(|(_, _, weight)| usable * weight / total_weight.max(0.001))
            .collect();

        let header = ui
            .allocate_exact_size(Vec2::new(width, 22.0), Sense::hover())
            .0;
        let painter = ui.painter();
        let mut x = header.left() + 8.0;
        painter.text(
            egui::pos2(x, header.center().y),
            Align2::LEFT_CENTER,
            "Title",
            FontId::proportional(12.0),
            Color32::GRAY,
        );
        x += title_width;
        for ((name, _, _), column_width) in visible.iter().zip(&widths) {
            painter.text(
                egui::pos2(x, header.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(12.0),
                Color32::GRAY,
            );
            x += column_width;
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &index in indices {
                    let entry = &self.library[index];
                    let selected = self.selected == Some(index);
                    let row_height = 24.0;
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_height),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.selected = Some(index);
                    }
                    if response.double_clicked() {
                        self.pending_play = Some(entry.clone());
                    }
                    if selected && self.controller_scroll {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Play").clicked() {
                            self.pending_play = Some(entry.clone());
                            ui.close_menu();
                        }
                        if ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(entry.source.display_path());
                            ui.close_menu();
                        }
                    });

                    let bg = if selected {
                        ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                    } else if response.hovered() {
                        ui.visuals().widgets.hovered.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 2.0, bg);
                    }

                    let painter = ui.painter().with_clip_rect(rect);
                    let y = rect.center().y;
                    let text_color = if selected {
                        ui.visuals().strong_text_color()
                    } else {
                        ui.visuals().text_color()
                    };
                    let mut x = rect.left() + 8.0;
                    painter.text(
                        egui::pos2(x, y),
                        Align2::LEFT_CENTER,
                        &entry.title,
                        FontId::proportional(13.0),
                        text_color,
                    );
                    x += title_width;
                    for ((_, bit, _), column_width) in visible.iter().zip(&widths) {
                        painter.text(
                            egui::pos2(x, y),
                            Align2::LEFT_CENTER,
                            column_cell(entry, *bit, self.play_stats.seconds(&entry.save_stem())),
                            FontId::proportional(12.0),
                            Color32::from_gray(170),
                        );
                        x += column_width;
                    }
                }
            });
    }

    fn grid(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        let columns = self.settings.grid_columns.clamp(1, 8) as usize;
        let gap = 8.0;
        let available = ui.available_width();
        let cell_width =
            ((available - gap * (columns.saturating_sub(1)) as f32) / columns as f32).max(80.0);
        let cell_height = (cell_width * 0.52).clamp(64.0, 140.0);
        ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &index in indices {
                        let entry = &self.library[index];
                        let selected = self.selected == Some(index);
                        let size = Vec2::new(cell_width, cell_height);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        if response.clicked() {
                            self.selected = Some(index);
                        }
                        if response.double_clicked() {
                            self.pending_play = Some(entry.clone());
                        }
                        if selected && self.controller_scroll {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        response.context_menu(|ui| {
                            if ui.button("Play").clicked() {
                                self.pending_play = Some(entry.clone());
                                ui.close_menu();
                            }
                        });
                        let bg = if selected {
                            ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                        } else if response.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().extreme_bg_color
                        };
                        ui.painter().rect_filled(rect, 4.0, bg);
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            Stroke::new(1.0_f32, Color32::from_gray(70)),
                            egui::StrokeKind::Inside,
                        );
                        let painter = ui.painter().with_clip_rect(rect.shrink(8.0));
                        painter.text(
                            rect.left_top() + Vec2::new(10.0, 12.0),
                            Align2::LEFT_TOP,
                            &entry.title,
                            FontId::proportional(13.0),
                            ui.visuals().strong_text_color(),
                        );
                        painter.text(
                            rect.left_bottom() + Vec2::new(10.0, -10.0),
                            Align2::LEFT_BOTTOM,
                            format!(
                                "{} · {} · {}{}",
                                entry.system(),
                                entry.mapper_label(),
                                entry.region_label(),
                                match self.play_stats.seconds(&entry.save_stem()) {
                                    0 => String::new(),
                                    seconds => format!(" · {}", format_duration(seconds)),
                                }
                            ),
                            FontId::proportional(10.0),
                            Color32::from_gray(160),
                        );
                    }
                });
            });
    }

    fn detached_windows(&self) -> bool {
        self.settings.detachable_windows && !cfg!(any(target_os = "android", target_os = "ios"))
    }

    fn rom_manager_window(&mut self, ctx: &egui::Context) {
        if !self.rom_manager.open {
            return;
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let id = egui::ViewportId::from_hash_of(("unko-rom-manager", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("ROM Manager — Unko")
                .with_min_inner_size([760.0, 480.0])
                .with_resizable(true);
            if !self.rom_manager_window_shown {
                builder = if let Some([x, y, width, height]) = self.settings.layout_rom_manager {
                    builder
                        .with_position([x, y])
                        .with_inner_size([width, height])
                } else {
                    builder
                        .with_position([80.0, 80.0])
                        .with_inner_size([1040.0, 720.0])
                };
            }
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.rom_manager.show_contents(ctx);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            self.rom_manager_window_shown = true;
            if let Some(layout) = layout {
                self.settings.layout_rom_manager = Some(layout);
            }
            if closed {
                self.rom_manager.open = false;
                self.rom_manager_window_shown = false;
                let _ = self.settings.save(&Settings::path());
            }
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let mut open = self.rom_manager.open;
            self.rom_manager.show_embedded(ctx, &mut open);
            self.rom_manager.open = open;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-settings", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Settings — Unko")
                .with_min_inner_size([520.0, 360.0])
                .with_resizable(true);
            if !self.settings_window_shown {
                builder = if let Some([x, y, width, height]) = self.settings.layout_settings {
                    builder
                        .with_position([x, y])
                        .with_inner_size([width, height])
                } else {
                    builder.with_inner_size([780.0, 540.0])
                };
            }
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.settings_frame(ctx, &mut open, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            self.settings_window_shown = true;
            if let Some(layout) = layout {
                self.settings.layout_settings = Some(layout);
            }
            if closed {
                open = false;
                self.settings_window_shown = false;
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.settings_frame(ctx, &mut open, true);
        }
        self.settings_open = open;
    }

    fn settings_frame(&mut self, ctx: &egui::Context, open: &mut bool, embedded: bool) {
        let mut settings = self.settings.clone();
        let mut tab = self.tab;
        let selected_entry = self
            .selected
            .and_then(|index| self.library.get(index))
            .cloned();
        let mut gamepads = self.gamepads.take();
        let gamepad_error = self.gamepad_error.clone();
        let mut hotkey_capture = self.hotkey_capture.take();
        let mut reset_requested = false;
        let mut layout_reset = false;
        let mut play_reset = false;

        let mut body = |ui: &mut egui::Ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.set_min_width(140.0);
                        ui.vertical(|ui| {
                            for entry in Tab::ALL {
                                if ui.selectable_label(tab == entry, entry.label()).clicked() {
                                    tab = entry;
                                }
                            }
                        });
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_min_width(560.0);
                            match tab {
                                Tab::General => {
                                    reset_requested = general_tab(
                                        ui,
                                        &mut settings,
                                        ctx,
                                        &mut layout_reset,
                                        &self.play_stats,
                                        &mut play_reset,
                                    );
                                }
                                Tab::Interface => interface_tab(ui, &mut settings),
                                Tab::Graphics => graphics_tab(ui, &mut settings, ctx),
                                Tab::Audio => audio_tab(
                                    ui,
                                    &mut settings,
                                    self.audio.as_ref(),
                                    self.audio_error.as_deref(),
                                ),
                                Tab::Controllers => controllers_tab(
                                    ui,
                                    &mut settings,
                                    gamepads.as_mut(),
                                    gamepad_error.as_deref(),
                                    &mut hotkey_capture,
                                ),
                                Tab::Paths => paths_tab(ui, &mut settings),
                                Tab::Nes => nes_tab(ui, &mut settings, selected_entry.as_ref()),
                                Tab::Advanced => advanced_tab(ui, &mut settings),
                            }
                        });
                    });
                });
        };

        if embedded {
            egui::Window::new("Settings")
                .open(open)
                .default_width(760.0)
                .default_height(480.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }

        self.gamepads = gamepads;
        self.hotkey_capture = hotkey_capture;
        if play_reset {
            self.play_stats = PlayStats::default();
            self.flush_play_stats();
        }
        self.tab = tab;
        if settings != self.settings {
            let old = self.settings.clone();
            self.settings = settings;
            self.apply_settings(ctx, &old);
        }
        if layout_reset {
            self.window_generation = self.window_generation.wrapping_add(1);
            self.settings_window_shown = false;
            self.devices_window_shown = false;
            self.cheats_window_shown = false;
            self.rom_manager_window_shown = false;
            self.debugger_window_shown = false;
        }
        if reset_requested {
            self.confirm_reset = true;
        }
    }

    fn cheats_window(&mut self, ctx: &egui::Context) {
        if !self.cheats_open {
            return;
        }
        let mut open = self.cheats_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-cheats", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Cheats — Unko")
                .with_min_inner_size([560.0, 360.0])
                .with_resizable(true);
            if !self.cheats_window_shown {
                builder = if let Some([x, y, width, height]) = self.settings.layout_cheats {
                    builder
                        .with_position([x, y])
                        .with_inner_size([width, height])
                } else {
                    builder.with_inner_size([700.0, 480.0])
                };
            }
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.cheats_frame(ctx, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            self.cheats_window_shown = true;
            if let Some(layout) = layout {
                self.settings.layout_cheats = Some(layout);
            }
            if closed {
                open = false;
                self.cheats_window_shown = false;
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.cheats_frame(ctx, true);
        }
        self.cheats_open = open;
    }

    fn cheats_frame(&mut self, ctx: &egui::Context, embedded: bool) {
        if self.player.is_none() {
            if embedded {
                egui::Window::new("Cheats")
                    .default_size([420.0, 120.0])
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Load a game to edit its cheats.");
                    });
            }
            return;
        }
        let mut entries = self.cheats.clone();
        let mut code = self.cheat_code.clone();
        let mut address = self.cheat_addr.clone();
        let mut value = self.cheat_value.clone();
        let mut error = self.cheat_error.clone();
        let mut add_genie = false;
        let mut add_ram = false;
        let mut changed = false;

        let mut body = |ui: &mut egui::Ui| {
            ui.heading("Cheats");
            ui.label(
                RichText::new(
                    "Game Genie codes patch PRG reads; raw RAM writes apply before each frame. \
                     Up to 32 entries, stored per game beside the save states.",
                )
                .weak()
                .small(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Game Genie");
                ui.add(
                    egui::TextEdit::singleline(&mut code)
                        .hint_text("SXIOPO")
                        .desired_width(140.0),
                );
                if ui
                    .add_enabled(entries.len() < 32, egui::Button::new("Add"))
                    .clicked()
                {
                    add_genie = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Raw RAM");
                ui.add(
                    egui::TextEdit::singleline(&mut address)
                        .hint_text("0010")
                        .desired_width(70.0),
                );
                ui.label("=");
                ui.add(
                    egui::TextEdit::singleline(&mut value)
                        .hint_text("99")
                        .desired_width(50.0),
                );
                if ui
                    .add_enabled(entries.len() < 32, egui::Button::new("Add"))
                    .clicked()
                {
                    add_ram = true;
                }
            });
            if let Some(error) = &error {
                ui.colored_label(Color32::from_rgb(0xE0, 0x80, 0x60), error);
            }
            ui.separator();
            if entries.is_empty() {
                ui.label(RichText::new("No cheats for this game.").weak());
            }
            let mut remove = None;
            for (index, entry) in entries.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut entry.enabled, "").changed() {
                        changed = true;
                    }
                    ui.label(entry.label());
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                entries.remove(index);
                changed = true;
            }
        };

        if embedded {
            egui::Window::new("Cheats")
                .default_width(680.0)
                .default_height(420.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }

        if add_genie {
            if entries.len() >= 32 {
                error = Some("32 cheats maximum".into());
            } else {
                match unko_nes::GameGenie::parse(&code) {
                    Ok(decoded) => {
                        entries.push(CheatEntry {
                            cheat: unko_core::Cheat::GameGenie {
                                code: decoded.code().to_string(),
                                address: decoded.address(),
                                value: decoded.value(),
                                compare: decoded.compare(),
                            },
                            enabled: true,
                        });
                        code.clear();
                        error = None;
                        changed = true;
                    }
                    Err(err) => error = Some(err.to_string()),
                }
            }
        }
        if add_ram {
            if entries.len() >= 32 {
                error = Some("32 cheats maximum".into());
            } else {
                let parse_address =
                    u16::from_str_radix(address.trim().trim_start_matches("0x"), 16);
                let parse_value = u8::from_str_radix(value.trim().trim_start_matches("0x"), 16);
                match (parse_address, parse_value) {
                    (Ok(addr), Ok(val)) if addr < 0x2000 => {
                        entries.push(CheatEntry {
                            cheat: unko_core::Cheat::Ram {
                                address: addr,
                                value: val,
                            },
                            enabled: true,
                        });
                        address.clear();
                        value.clear();
                        error = None;
                        changed = true;
                    }
                    (Ok(_), Ok(_)) => {
                        error = Some("Raw RAM cheats use $0000-$07FF".into());
                    }
                    _ => error = Some("Address and value must be hexadecimal".into()),
                }
            }
        }

        self.cheats = entries;
        self.cheat_code = code;
        self.cheat_addr = address;
        self.cheat_value = value;
        self.cheat_error = error;
        if changed {
            self.apply_cheats();
            self.save_cheats();
        }
    }

    fn devices_window(&mut self, ctx: &egui::Context) {
        if !self.devices_open {
            return;
        }
        let mut open = self.devices_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-devices", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Devices — Unko")
                .with_min_inner_size([420.0, 320.0])
                .with_resizable(true);
            if !self.devices_window_shown {
                builder = if let Some([x, y, width, height]) = self.settings.layout_devices {
                    builder
                        .with_position([x, y])
                        .with_inner_size([width, height])
                } else {
                    builder.with_inner_size([680.0, 520.0])
                };
            }
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.devices_frame(ctx, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            self.devices_window_shown = true;
            if let Some(layout) = layout {
                self.settings.layout_devices = Some(layout);
            }
            if closed {
                open = false;
                self.devices_window_shown = false;
                if let Some(note) = self.piano_last.take() {
                    if let Some(player) = &mut self.player {
                        player.set_piano_note(note, false);
                    }
                }
                if let Some(player) = &mut self.player {
                    for (note, held) in self.piano_key_held.iter_mut().enumerate() {
                        if *held {
                            player.set_piano_note(note as u8, false);
                            *held = false;
                        }
                    }
                }
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.devices_frame(ctx, true);
        }
        self.devices_open = open;
    }

    fn devices_frame(&mut self, ctx: &egui::Context, embedded: bool) {
        let names = self.gamepads.as_ref().map(Gamepads::names);
        let empty_names = [None, None, None, None];
        let pad_names = names.as_ref().unwrap_or(&empty_names);
        let connected = self.gamepads.as_ref().map_or(0, Gamepads::connected);
        let gamepad_error = self.gamepad_error.clone();
        let mut settings = self.settings.clone();
        let mut microphone = self.microphone;
        let mut datach_code = self.datach_code.clone();
        let mut datach_scan = false;
        let has_player = self.player.is_some();
        let karaoke = self
            .player
            .as_ref()
            .is_some_and(|player| player.mapper() == 188);
        let microphone_error = self.microphone_error.clone();
        let microphone_devices = self.microphone_devices.clone();
        let mut piano_last = self.piano_last;
        let mut piano_events = Vec::new();
        let vs_system = self
            .player
            .as_ref()
            .is_some_and(|player| player.is_vs_system());
        let mut vs_service = self.vs_service;
        let mut vs_coin_events = Vec::new();
        let mut vs_dips = self.vs_dips;
        let mut vs_primary = self.vs_primary;
        let mut body = |ui: &mut egui::Ui| {
            ui.heading("NES ports");
            ui.label(
                RichText::new(
                    "The two physical ports carry players 1/2, or players 1-4 \
                     with a Four Score adapter. A disabled port ignores \
                     keyboard, touch and gamepad input.",
                )
                .weak()
                .small(),
            );
            for port in 0..2 {
                let device = if port == 0 {
                    settings.port1_device
                } else {
                    settings.port2_device
                };
                let input = if port == 0 {
                    settings.port1_input
                } else {
                    settings.port2_input
                };
                ui.horizontal(|ui| {
                    ui.checkbox(&mut settings.port_disabled[port], "Disabled");
                    ui.label(format!(
                        "Port {}: {} — {}",
                        port + 1,
                        port_input_description(input, pad_names),
                        device_label(device)
                    ));
                });
            }
            if settings.port1_device == PortDevice::Microphone
                || settings.port2_device == PortDevice::Microphone
                || karaoke
            {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!settings.microphone_input, |ui| {
                        ui.checkbox(
                            &mut microphone,
                            "Microphone input (hold while making noise)",
                        );
                    });
                    ui.checkbox(&mut settings.microphone_input, "Use system microphone");
                });
                if settings.microphone_input {
                    ui.horizontal(|ui| {
                        ui.label("Input");
                        let selected = if settings.microphone_device.is_empty() {
                            "System default".to_string()
                        } else {
                            settings.microphone_device.clone()
                        };
                        egui::ComboBox::from_id_salt("microphone-device")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut settings.microphone_device,
                                    String::new(),
                                    "System default",
                                );
                                for name in &microphone_devices {
                                    ui.selectable_value(
                                        &mut settings.microphone_device,
                                        name.clone(),
                                        name,
                                    );
                                }
                            });
                    });
                    ui.add(
                        egui::Slider::new(&mut settings.microphone_threshold, 0.002..=0.2)
                            .logarithmic(true)
                            .text("Mic threshold (peak)"),
                    );
                    ui.label(
                        RichText::new(
                            "System input uses the selected peak threshold with a short \
                             hold; the analog ADC is not modelled.",
                        )
                        .weak()
                        .small(),
                    );
                }
                if let Some(err) = &microphone_error {
                    ui.label(RichText::new(err).weak().small());
                }
                if karaoke {
                    ui.label(
                        RichText::new(
                            "Karaoke Studio: the cartridge microphone maps here; A/B \
                             buttons come from controller port 1.",
                        )
                        .weak()
                        .small(),
                    );
                }
            }
            if settings.port1_device == PortDevice::MiraclePiano
                || settings.port2_device == PortDevice::MiraclePiano
            {
                ui.add_space(10.0);
                ui.separator();
                ui.heading("Miracle Piano");
                ui.label(
                    RichText::new(
                        "Click or drag the keys, or use the number/QWERTY/ASDFG/ZXCVB \
                         rows when computer keyboard input is on. Notes are sent on \
                         channel 0; the game's controller belongs in the other port.",
                    )
                    .weak()
                    .small(),
                );
                ui.checkbox(&mut settings.piano_keyboard, "Computer keyboard input");
                piano_events = crate::piano::keyboard(ui, &mut piano_last);
            }
            if vs_system {
                ui.add_space(10.0);
                ui.separator();
                ui.heading("Vs. System");
                ui.label(
                    RichText::new(
                        "Each coin press closes the acceptor contact for about \
                         67 ms, as on the arcade harness. DIP 1-8 and the \
                         primary-CPU bit feed $4016/$4017.",
                    )
                    .weak()
                    .small(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Coin 1").clicked() {
                        vs_coin_events.push(0);
                    }
                    if ui.button("Coin 2").clicked() {
                        vs_coin_events.push(1);
                    }
                    ui.checkbox(&mut vs_service, "Service");
                    ui.checkbox(&mut vs_primary, "Primary CPU");
                });
                egui::Grid::new("vs-dips")
                    .num_columns(4)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for index in 0..8 {
                            let mut set = (vs_dips >> index) & 1 != 0;
                            if ui
                                .checkbox(&mut set, format!("DIP {}", index + 1))
                                .changed()
                            {
                                if set {
                                    vs_dips |= 1 << index;
                                } else {
                                    vs_dips &= !(1 << index);
                                }
                            }
                            if index % 4 == 3 {
                                ui.end_row();
                            }
                        }
                    });
            }
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Gamepads");
            match &names {
                Some(names) => {
                    egui::Grid::new("devices-pads")
                        .num_columns(3)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Pad");
                            ui.label("NES player");
                            ui.label("Disabled");
                            ui.end_row();
                            for (slot, name) in names.iter().enumerate() {
                                ui.label(match name {
                                    Some(name) => format!("{}: {name}", slot + 1),
                                    None => format!("{}: (not connected)", slot + 1),
                                });
                                egui::ComboBox::from_id_salt(("pad-port", slot))
                                    .selected_text(format!(
                                        "Player {}",
                                        settings.pad_ports[slot] + 1
                                    ))
                                    .show_ui(ui, |ui| {
                                        for port in 0..4u8 {
                                            ui.selectable_value(
                                                &mut settings.pad_ports[slot],
                                                port,
                                                format!("Player {}", port + 1),
                                            );
                                        }
                                    });
                                ui.checkbox(&mut settings.pad_disabled[slot], "");
                                ui.end_row();
                            }
                        });
                    ui.label(
                        RichText::new(format!(
                            "{connected} connected. Two pads can drive the same \
                             player; their buttons are merged."
                        ))
                        .weak()
                        .small(),
                    );
                }
                None => {
                    ui.colored_label(
                        Color32::from_rgb(0xE0, 0x80, 0x60),
                        gamepad_error
                            .as_deref()
                            .unwrap_or("SDL3 gamepad backend unavailable"),
                    );
                    ui.label("Keyboard and the touch overlay still work.");
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Sources");
            ui.label(
                "Auto: keyboard/touch on player 1 plus the gamepad matrix. \
                 Explicit Connected choices take precedence.",
            );
            ui.label("Gamepads: routed through the matrix above when Connected is Auto.");
            ui.label("Zapper, paddle, mouse, Power Pad and Family BASIC use the port's device.");
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Datach barcode");
            ui.label(
                RichText::new(
                    "Mapper 157 only. Enter a 12-digit UPC-A or 13-digit EAN-13 code; the \
                     scanner returns it as the documented bar pattern.",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut datach_code)
                        .hint_text("012345678905")
                        .desired_width(190.0),
                );
                if ui
                    .add_enabled(has_player, egui::Button::new("Scan"))
                    .clicked()
                {
                    datach_scan = true;
                }
            });
            if settings.port1_device == PortDevice::MiraclePiano
                || settings.port2_device == PortDevice::MiraclePiano
            {
                ui.label(
                    RichText::new(
                        "Miracle Piano: the MIDI transport is modelled; received bytes are \
                         injected by tests because the piano firmware is not available.",
                    )
                    .weak()
                    .small(),
                );
            }
        };

        if embedded {
            egui::Window::new("Devices")
                .default_width(660.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }
        self.microphone = microphone;
        self.piano_last = piano_last;
        self.vs_service = vs_service;
        self.vs_dips = vs_dips;
        self.vs_primary = vs_primary;
        if let Some(player) = &mut self.player {
            player.set_vs_input(vs_service, vs_dips, vs_primary);
            for slot in vs_coin_events {
                player.insert_vs_coin(slot);
            }
        }
        if !piano_events.is_empty() {
            if let Some(player) = &mut self.player {
                for (note, on) in piano_events {
                    player.set_piano_note(note, on);
                }
            }
        }
        let piano_selected = settings.port1_device == PortDevice::MiraclePiano
            || settings.port2_device == PortDevice::MiraclePiano;
        let mut key_events = Vec::new();
        if piano_selected && settings.piano_keyboard && !ctx.wants_keyboard_input() {
            for (key, note) in crate::piano::KEY_NOTES {
                let down = ctx.input(|input| input.key_down(*key));
                if down != self.piano_key_held[usize::from(*note)] {
                    self.piano_key_held[usize::from(*note)] = down;
                    key_events.push((*note, down));
                }
            }
        } else {
            for (note, held) in self.piano_key_held.iter_mut().enumerate() {
                if *held {
                    *held = false;
                    key_events.push((note as u8, false));
                }
            }
        }
        if !key_events.is_empty() {
            if let Some(player) = &mut self.player {
                for (note, on) in key_events {
                    player.set_piano_note(note, on);
                }
            }
        }
        self.datach_code = datach_code;
        if datach_scan {
            let result = self
                .player
                .as_mut()
                .map(|player| player.set_datach_barcode(&self.datach_code));
            match result {
                Some(true) => self.set_status("Datach barcode scanned"),
                Some(false) => self.set_status("Barcode invalid or not a Datach cartridge"),
                None => {}
            }
        }
        if settings != self.settings {
            let old = self.settings.clone();
            self.settings = settings;
            self.apply_settings(ctx, &old);
        }
    }

    fn reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Reset all settings?")
            .collapsible(false)
            .default_size([460.0, 180.0])
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "This restores configuration, window layout, hotkeys, gamepad \
                     bindings, per-port devices and per-game input profiles, and \
                     removes saved state thumbnails.",
                );
                ui.label(
                    RichText::new("Save states, SRAM and recordings are never deleted.").strong(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });
        if confirmed {
            self.confirm_reset = false;
            self.reset_all_settings(ctx);
        } else if !open {
            self.confirm_reset = false;
        }
    }

    fn reset_all_settings(&mut self, ctx: &egui::Context) {
        let old = self.settings.clone();
        // Per-game profiles and state thumbnails are regenerable; state files,
        // SRAM and recordings are not touched.
        if let Ok(entries) = std::fs::read_dir(&old.save_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let extension = path.extension().and_then(|value| value.to_str());
                if matches!(extension, Some("pad" | "thumb")) {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        self.settings = Settings::default();
        self.active_mapping = self.settings.pad_mapping;
        self.tab = Tab::General;
        self.window_generation = self.window_generation.wrapping_add(1);
        self.settings_window_shown = false;
        self.devices_window_shown = false;
        self.cheats_window_shown = false;
        self.rom_manager_window_shown = false;
        self.debugger_window_shown = false;
        self.apply_settings(ctx, &old);
        self.scan();
        self.set_status("Reset all settings to defaults");
    }

    fn netplay_apply_defaults(&mut self) {
        if self.netplay_host {
            self.netplay_local_port = "7777".into();
            self.netplay_peer = "127.0.0.1:7778".into();
        } else {
            self.netplay_local_port = "7778".into();
            self.netplay_peer = "127.0.0.1:7777".into();
        }
    }

    fn start_netplay(&mut self) {
        let Some(player) = &mut self.player else {
            self.netplay_status = "Load a game before starting netplay.".into();
            return;
        };
        let players = self.netplay_players.clamp(2, 4);
        let slot = self.netplay_slot.clamp(1, players) - 1;
        let Ok(port) = self.netplay_local_port.trim().parse::<u16>() else {
            self.netplay_status = "Local port must be a number.".into();
            return;
        };
        let config = SessionConfig {
            buffer_frames: 8,
            input_delay: self.netplay_delay,
            prediction: self.netplay_prediction,
            players,
            local_player: slot,
            hash_states: true,
        };
        let relay = self.netplay_relay_server.trim().to_string();
        let room = self.netplay_room.trim().to_string();
        if !self.netplay_hole_punch && !relay.is_empty() {
            if room.is_empty() {
                self.netplay_status = "Relay rooms need a name.".into();
                return;
            }
            match RelayTransport::bind(&format!("0.0.0.0:{port}"), &relay, &room) {
                Ok(transport) => {
                    player.start_netplay_with(Box::new(transport), config);
                    self.netplay_status =
                        format!("Relay room {room} on {relay}, UDP {port} local.");
                }
                Err(err) => self.netplay_status = format!("Relay setup failed: {err}"),
            }
            return;
        }
        let peers: Vec<String> = self
            .netplay_peer
            .split(',')
            .map(str::trim)
            .filter(|peer| !peer.is_empty())
            .map(str::to_string)
            .collect();
        if peers.len() != usize::from(players - 1) {
            self.netplay_status = format!(
                "{players} players need exactly {} peer address(es), found {}.",
                players - 1,
                peers.len()
            );
            return;
        }
        if self.netplay_hole_punch {
            let mut endpoints = Vec::with_capacity(peers.len());
            for peer in &peers {
                match peer.parse::<std::net::SocketAddr>() {
                    Ok(endpoint) => endpoints.push(endpoint),
                    Err(err) => {
                        self.netplay_status =
                            format!("Hole punch needs public ip:port peers ({peer}: {err}).");
                        return;
                    }
                }
            }
            match HolePunchTransport::bind(&format!("0.0.0.0:{port}"), &endpoints) {
                Ok(transport) => {
                    self.netplay_status =
                        format!("Hole punching UDP {port} to {} peer(s)…", endpoints.len());
                    self.netplay_pending_punch = Some((transport, config, port));
                    self.netplay_punch_deadline = Some(Instant::now() + Duration::from_secs(6));
                    // Do not advance the console while the peers agree on
                    // connectivity: both sides must start the session from
                    // the same frame.
                    self.netplay_resume_after_punch = !player.paused;
                    player.paused = true;
                }
                Err(err) => self.netplay_status = format!("Bind failed: {err}"),
            }
            return;
        }
        let transport = UdpMesh::bind(&format!("0.0.0.0:{port}")).and_then(|mut mesh| {
            for peer in &peers {
                mesh.add_peer(peer)?;
            }
            Ok(mesh)
        });
        match transport {
            Ok(mesh) => {
                player.start_netplay_with(Box::new(mesh), config);
                self.netplay_status = format!(
                    "{} on UDP {port}, {} player(s), slot {}, peer(s) {}",
                    if self.netplay_host {
                        "Hosting"
                    } else {
                        "Joined"
                    },
                    players,
                    slot + 1,
                    peers.join(", ")
                );
            }
            Err(err) => self.netplay_status = format!("Netplay setup failed: {err}"),
        }
    }

    /// Detects a public endpoint in a worker thread.
    ///
    /// Tries a UPnP-IGD mapping first (which also opens the port). When the
    /// router has no UPnP, falls back to a STUN binding request for the
    /// external IP; the port then still has to be forwarded manually.
    fn spawn_nat_detect(&mut self) {
        let Ok(port) = self.netplay_local_port.trim().parse::<u16>() else {
            self.netplay_upnp_status = "Local port must be a number.".into();
            return;
        };
        if self.netplay_upnp_rx.is_some() {
            return;
        }
        let peer = self
            .netplay_peer
            .split(',')
            .map(str::trim)
            .find(|peer| !peer.is_empty())
            .unwrap_or("127.0.0.1:0")
            .to_string();
        let stun_server = self.netplay_stun_server.trim().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.netplay_upnp_rx = Some(rx);
        self.netplay_upnp_status = "Looking for a UPnP gateway, then STUN…".into();
        std::thread::spawn(move || {
            let local = local_address_toward(&peer);
            let upnp = (|| -> Result<String, String> {
                let device = discover(Duration::from_secs(2)).map_err(|err| err.to_string())?;
                let ip = external_ip(&device).map_err(|err| err.to_string())?;
                let request = MappingRequest::new(&local, port, MappingProtocol::Udp);
                add_port_mapping(&device, &request).map_err(|err| err.to_string())?;
                Ok(endpoint(&ip, port))
            })();
            let result = match upnp {
                Ok(public) => Ok((public, "UPnP mapping active".to_string())),
                Err(upnp_error) => {
                    if stun_server.is_empty() {
                        Err(format!("UPnP: {upnp_error}"))
                    } else {
                        match stun_public_address(&stun_server, Duration::from_secs(3)) {
                            Ok(address) => Ok((
                                format!("{}:{port}", address.ip()),
                                "STUN external IP; forward this UDP port on the router".to_string(),
                            )),
                            Err(stun_error) => {
                                Err(format!("UPnP: {upnp_error}; STUN: {stun_error}"))
                            }
                        }
                    }
                }
            };
            let _ = tx.send(result);
        });
    }

    /// Advances a pending hole-punch handshake and starts the session once
    /// every peer is confirmed.
    fn poll_hole_punch(&mut self) {
        let Some((transport, _, _)) = self.netplay_pending_punch.as_mut() else {
            return;
        };
        let relay = self.netplay_relay_server.trim().to_string();
        let room = self.netplay_room.trim().to_string();
        let expired = self
            .netplay_punch_deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if expired && !relay.is_empty() {
            let (_, config, port) = self
                .netplay_pending_punch
                .take()
                .expect("pending hole punch");
            self.netplay_punch_deadline = None;
            match RelayTransport::bind(&format!("0.0.0.0:{port}"), &relay, &room) {
                Ok(relay_transport) => {
                    if let Some(player) = &mut self.player {
                        player.start_netplay_with(Box::new(relay_transport), config);
                        player.paused = !self.netplay_resume_after_punch;
                    }
                    self.netplay_status =
                        format!("Hole punch timed out; relaying via {relay} room {room}.");
                }
                Err(err) => {
                    self.netplay_status =
                        format!("Hole punch failed and relay setup failed: {err}");
                }
            }
            return;
        }
        transport.punch();
        if let Some(player) = &mut self.player {
            // Keep the console frozen for the whole handshake even if the
            // user toggles pause in the UI.
            player.paused = true;
        }
        let confirmed = transport.confirmed();
        let total = transport.peers().len();
        if transport.connected() {
            let (transport, config, _) = self
                .netplay_pending_punch
                .take()
                .expect("pending hole punch");
            self.netplay_punch_deadline = None;
            if let Some(player) = &mut self.player {
                player.start_netplay_with(Box::new(transport), config);
                player.paused = !self.netplay_resume_after_punch;
                self.netplay_status = format!("Hole punch confirmed ({total}/{total}); running.");
            }
            if self.netplay_upnp {
                self.spawn_nat_detect();
            }
        } else {
            self.netplay_status = format!("Hole punching… {confirmed}/{total} peer(s) confirmed.");
        }
    }

    /// Polls the NAT worker without blocking the frame.
    fn poll_nat(&mut self) {
        let Some(rx) = self.netplay_upnp_rx.as_ref() else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("UPnP worker stopped".into()))
            }
        };
        if let Some(result) = outcome {
            self.netplay_upnp_rx = None;
            match result {
                Ok((public, detail)) => {
                    self.netplay_upnp_status = format!("{detail}. Public endpoint {public}");
                    self.netplay_public_endpoint = public;
                }
                Err(err) => self.netplay_upnp_status = format!("NAT detection failed: {err}"),
            }
        }
    }

    fn netplay_window(&mut self, ctx: &egui::Context) {
        if !self.netplay_open {
            self.poll_nat();
            return;
        }
        self.poll_nat();
        let mut open = self.netplay_open;
        let active = self
            .player
            .as_ref()
            .is_some_and(crate::player::Player::netplay_active);
        egui::Window::new("NetPlay")
            .open(&mut open)
            .default_size([560.0, 420.0])
            .show(ctx, |ui| {
                let role_before = self.netplay_host;
                ui.horizontal(|ui| {
                    ui.label("Role");
                    ui.add_enabled_ui(!active, |ui| {
                        if ui.selectable_label(self.netplay_host, "Host").clicked() {
                            self.netplay_host = true;
                        }
                        if ui.selectable_label(!self.netplay_host, "Join").clicked() {
                            self.netplay_host = false;
                        }
                    });
                });
                if self.netplay_host != role_before {
                    self.netplay_apply_defaults();
                }
                ui.horizontal(|ui| {
                    ui.label("Players");
                    let players_before = self.netplay_players;
                    egui::ComboBox::from_id_salt("netplay-players")
                        .selected_text(self.netplay_players.to_string())
                        .show_ui(ui, |ui| {
                            ui.add_enabled_ui(!active, |ui| {
                                for count in 2..=4u8 {
                                    ui.selectable_value(
                                        &mut self.netplay_players,
                                        count,
                                        count.to_string(),
                                    );
                                }
                            });
                        });
                    if self.netplay_players != players_before {
                        self.netplay_slot = self.netplay_slot.min(self.netplay_players);
                    }
                    ui.label("My slot");
                    egui::ComboBox::from_id_salt("netplay-slot")
                        .selected_text(self.netplay_slot.to_string())
                        .show_ui(ui, |ui| {
                            ui.add_enabled_ui(!active, |ui| {
                                for slot in 1..=self.netplay_players {
                                    ui.selectable_value(&mut self.netplay_slot, slot, slot.to_string());
                                }
                            });
                        });
                    ui.label("Local port");
                    ui.add_enabled(
                        !active,
                        egui::TextEdit::singleline(&mut self.netplay_local_port)
                            .desired_width(72.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Peers");
                    ui.add_enabled(
                        !active,
                        egui::TextEdit::singleline(&mut self.netplay_peer).desired_width(240.0),
                    );
                    ui.label("Public");
                    ui.add_enabled(
                        !active,
                        egui::TextEdit::singleline(&mut self.netplay_public_endpoint)
                            .hint_text("ip:port")
                            .desired_width(120.0),
                    );
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !active && self.netplay_upnp_rx.is_none(),
                            egui::Button::new("Detect public"),
                        )
                        .clicked()
                    {
                        self.spawn_nat_detect();
                    }
                    ui.add_enabled(
                        !active && self.netplay_pending_punch.is_none(),
                        egui::Checkbox::new(&mut self.netplay_upnp, "Map on start"),
                    );
                    ui.add_enabled(
                        !active && self.netplay_pending_punch.is_none(),
                        egui::Checkbox::new(&mut self.netplay_hole_punch, "Hole punch"),
                    );
                    if self.netplay_upnp_rx.is_some() {
                        ui.spinner();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("STUN server");
                    ui.add_enabled(
                        !active,
                        egui::TextEdit::singleline(&mut self.netplay_stun_server)
                            .hint_text("host:port")
                            .desired_width(240.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Relay");
                    ui.add_enabled(
                        !active && self.netplay_pending_punch.is_none(),
                        egui::TextEdit::singleline(&mut self.netplay_relay_server)
                            .hint_text("host:port")
                            .desired_width(200.0),
                    );
                    ui.label("Room");
                    ui.add_enabled(
                        !active && self.netplay_pending_punch.is_none(),
                        egui::TextEdit::singleline(&mut self.netplay_room).desired_width(100.0),
                    );
                });
                if !self.netplay_upnp_status.is_empty() {
                    ui.label(&self.netplay_upnp_status);
                }
                ui.label(
                    RichText::new(
                        "Each player picks a unique slot and lists the other players' \
                         addresses (comma separated). Prediction fills missing input; \
                         delay reduces rollbacks. Rewind, movies and run-ahead are off.",
                    )
                    .small()
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.label("Prediction");
                    egui::ComboBox::from_id_salt("netplay-prediction")
                        .selected_text(match self.netplay_prediction {
                            PredictionMode::LastInput => "Last input",
                            PredictionMode::Neutral => "Neutral",
                        })
                        .show_ui(ui, |ui| {
                            for mode in [PredictionMode::LastInput, PredictionMode::Neutral] {
                                if ui
                                    .selectable_value(
                                        &mut self.netplay_prediction,
                                        mode,
                                        match mode {
                                            PredictionMode::LastInput => "Last input",
                                            PredictionMode::Neutral => "Neutral",
                                        },
                                    )
                                    .changed()
                                {
                                    if let Some(player) = &mut self.player {
                                        player.set_netplay_prediction(mode);
                                    }
                                }
                            }
                        });
                    ui.label("Input delay");
                    let response = ui.add(
                        egui::DragValue::new(&mut self.netplay_delay)
                            .range(0..=16)
                            .suffix(" frames"),
                    );
                    if response.changed() {
                        if let Some(player) = &mut self.player {
                            player.set_netplay_input_delay(self.netplay_delay);
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let can_start =
                        !active && self.netplay_pending_punch.is_none() && self.player.is_some();
                    if ui
                        .add_enabled(can_start, egui::Button::new("Start"))
                        .clicked()
                    {
                        self.start_netplay();
                    }
                    let punching = self.netplay_pending_punch.is_some();
                    if ui
                        .add_enabled(active || punching, egui::Button::new("Stop"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.stop_netplay();
                        }
                        self.netplay_pending_punch = None;
                        self.netplay_punch_deadline = None;
                        if let Some(player) = &mut self.player {
                            player.paused = !self.netplay_resume_after_punch;
                        }
                        self.netplay_status = "Stopped".into();
                    }
                });
                if !self.netplay_status.is_empty() {
                    ui.label(&self.netplay_status);
                }
                if let Some((frame, sent, received, rollbacks)) = self
                    .player
                    .as_ref()
                    .and_then(crate::player::Player::netplay_stats)
                {
                    let (config, session) = self
                        .player
                        .as_ref()
                        .and_then(crate::player::Player::netplay_session_stats)
                        .unwrap_or_default();
                    ui.label(format!(
                        "Frame {frame} · sent {sent} · received {received} · rollbacks {rollbacks}"
                    ));
                    ui.label(format!(
                        "Predicted {} · deepest rollback {} · delay {} · {} · {}/{} players, slot {}",
                        session.predicted,
                        session.max_rollback_depth,
                        config.input_delay,
                        config.prediction.name(),
                        config.players,
                        config.players,
                        config.local_player + 1,
                    ));
                    let (checked, mismatches, last) = self
                        .player
                        .as_ref()
                        .and_then(crate::player::Player::netplay_sync_stats)
                        .unwrap_or((0, 0, None));
                    ui.label(format!(
                        "State hashes: {checked} checked · {mismatches} mismatch(es)"
                    ));
                    if let Some(info) = last {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            format!("{info} — repairing state"),
                        );
                    }
                    if self
                        .player
                        .as_ref()
                        .is_some_and(crate::player::Player::netplay_resyncing)
                    {
                        ui.label("Transferring a save state from the authoritative peer…");
                    }
                    if let Some(err) = self
                        .player
                        .as_ref()
                        .and_then(crate::player::Player::netplay_error)
                    {
                        ui.colored_label(Color32::LIGHT_RED, err);
                    }
                }
            });
        self.netplay_open = open;
    }

    fn play_stats_path(&self) -> PathBuf {
        self.settings.save_dir.join(PlayStats::FILE_NAME)
    }

    fn flush_play_stats(&mut self) {
        if let Err(err) = self.play_stats.save(&self.play_stats_path()) {
            self.set_status(format!("Play time save failed: {err}"));
        }
    }

    fn game_props_path(&self, entry: &GameEntry) -> PathBuf {
        self.settings
            .save_dir
            .join(format!("{}.game", entry.save_stem()))
    }

    fn save_game_properties(&mut self, entry: &GameEntry) {
        let path = self.game_props_path(entry);
        self.status = match self.game_props.save(&path) {
            Ok(()) => format!("Saved game properties for {}", entry.title),
            Err(err) => format!("Could not save game properties: {err}"),
        };
    }

    fn reload_with_properties(&mut self, entry: &GameEntry) {
        self.save_game_properties(entry);
        self.stop_game();
        self.start_entry(entry.clone());
    }

    fn game_properties_window(&mut self, ctx: &egui::Context) {
        if !self.game_props_open {
            return;
        }
        let Some(entry) = self.game_props_entry.clone() else {
            self.game_props_open = false;
            return;
        };
        let mut open = self.game_props_open;
        let mut properties = self.game_props;
        let global = self.settings.region;
        let global_port1 = self.settings.port1_device;
        let global_port2 = self.settings.port2_device;
        let mut save = false;
        let mut reload = false;
        let mut reset_play = false;
        let played = self.play_stats.seconds(&entry.save_stem());
        egui::Window::new("Game properties")
            .open(&mut open)
            .default_size([480.0, 320.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(&entry.title).strong());
                ui.label(RichText::new(&entry.file_name).small().weak());
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Region");
                            egui::ComboBox::from_id_salt("game-props-region")
                                .selected_text(region_text(properties.region, global))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut properties.region,
                                        None,
                                        format!("Use global ({})", region_name(global)),
                                    );
                                    ui.selectable_value(
                                        &mut properties.region,
                                        Some(Region::Ntsc),
                                        "NTSC",
                                    );
                                    ui.selectable_value(
                                        &mut properties.region,
                                        Some(Region::Pal),
                                        "PAL",
                                    );
                                    ui.selectable_value(
                                        &mut properties.region,
                                        Some(Region::Dendy),
                                        "Dendy",
                                    );
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.label("Port 1 device");
                            egui::ComboBox::from_id_salt("game-props-port1")
                                .selected_text(port_text(properties.port1, global_port1))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut properties.port1,
                                        None,
                                        format!("Use global ({})", port_device_label(global_port1)),
                                    );
                                    for device in PORT_DEVICES {
                                        ui.selectable_value(
                                            &mut properties.port1,
                                            Some(device),
                                            port_device_label(device),
                                        );
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.label("Port 2 device");
                            egui::ComboBox::from_id_salt("game-props-port2")
                                .selected_text(port_text(properties.port2, global_port2))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut properties.port2,
                                        None,
                                        format!("Use global ({})", port_device_label(global_port2)),
                                    );
                                    for device in PORT_DEVICES {
                                        ui.selectable_value(
                                            &mut properties.port2,
                                            Some(device),
                                            port_device_label(device),
                                        );
                                    }
                                });
                        });
                        ui.add_space(6.0);
                        let mut override_mapper = properties.mapper.is_some();
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut override_mapper, "Override header mapper")
                                .changed()
                            {
                                properties.mapper = if override_mapper {
                                    Some(properties.mapper.unwrap_or(0))
                                } else {
                                    None
                                };
                            }
                            if let Some(mapper) = &mut properties.mapper {
                                ui.add(egui::DragValue::new(mapper).range(0..=4095));
                            }
                        });
                        ui.label(
                            RichText::new(
                                "The mapper override rewrites the iNES/NES 2.0 header. \
                                 iNES images accept 0-255; NES 2.0 images accept 0-4095. \
                                 FDS images are not affected.",
                            )
                            .small()
                            .weak(),
                        );
                        ui.label(
                            RichText::new(
                                "Overrides are stored next to the game's saves and apply \
                                 on the next load.",
                            )
                            .small()
                            .weak(),
                        );
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Play time");
                    ui.label(format_duration(played));
                    if ui.button("Reset play time").clicked() {
                        reset_play = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Save and reload").clicked() {
                        reload = true;
                    }
                    if ui.button("Reset").clicked() {
                        properties = GameProperties::default();
                        save = true;
                    }
                });
            });
        self.game_props_open = open;
        self.game_props = properties;
        if reset_play {
            self.play_stats.reset(&entry.save_stem());
            self.flush_play_stats();
        }
        if reload {
            self.reload_with_properties(&entry);
        } else if save {
            self.save_game_properties(&entry);
        }
    }

    fn slot_window(&mut self, ctx: &egui::Context) {
        if !self.slot_manager_open {
            return;
        }
        let mut open = self.slot_manager_open;
        let Some(player) = self.player.as_ref() else {
            egui::Window::new("Save states")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Load a game to manage its save-state slots.");
                });
            self.slot_manager_open = open;
            self.slot_thumbnails.clear();
            return;
        };
        let stem = player.stem.clone();
        let title = player.title.clone();
        let current = player.slot;
        let infos = player.slot_infos();
        let mut textures = std::mem::take(&mut self.slot_thumbnails);
        let mut action: Option<(SlotAction, u8, String)> = None;
        let mut renames: Vec<(u8, String)> = Vec::new();
        let mut clear_confirm = self.slot_clear_confirm;
        egui::Window::new("Save states")
            .open(&mut open)
            .default_size([620.0, 560.0])
            .max_height(620.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(RichText::new(&title).strong());
                ui.label(
                    RichText::new(
                        "Names are stored in a .meta sidecar beside each state.                          Deleting removes the state, thumbnail and metadata.",
                    )
                    .small()
                    .weak(),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(430.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for info in &infos {
                            ui.horizontal(|ui| {
                                if info.exists && info.has_thumb {
                                    match player.thumbnail(info.slot) {
                                        Some(bytes) if bytes.len() == 64 * 60 * 4 => {
                                            let key =
                                                (stem.clone(), info.slot, info.saved_unix);
                                            let texture =
                                                textures.entry(key).or_insert_with(|| {
                                                    let image =
                                                        egui::ColorImage::from_rgba_unmultiplied(
                                                            [64, 60], &bytes,
                                                        );
                                                    ctx.load_texture(
                                                        format!(
                                                            "{stem}-slot-{}-{}",
                                                            info.slot, info.saved_unix
                                                        ),
                                                        image,
                                                        egui::TextureOptions::NEAREST,
                                                    )
                                                });
                                            ui.image((texture.id(), egui::vec2(64.0, 60.0)));
                                        }
                                        _ => slot_placeholder(ui, "bad thumb"),
                                    }
                                } else {
                                    slot_placeholder(
                                        ui,
                                        if info.exists { "no thumb" } else { "empty" },
                                    );
                                }
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("#{}", info.slot));
                                        if info.exists {
                                            let mut name = info.name.clone();
                                            let response = ui.add(
                                                egui::TextEdit::singleline(&mut name)
                                                    .desired_width(220.0),
                                            );
                                            if response.changed() {
                                                renames.push((info.slot, name));
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new(default_slot_name(info.slot)).weak(),
                                            );
                                        }
                                        if info.slot == current {
                                            ui.label(RichText::new("active").small().strong());
                                        }
                                    });
                                    if info.exists {
                                        let when = modified_label(Some(info.saved_unix));
                                        let size = size_label(info.bytes);
                                        let detail = if info.frame > 0 {
                                            format!("frame {} · {when} · {size}", info.frame)
                                        } else {
                                            format!("{when} · {size}")
                                        };
                                        ui.label(RichText::new(detail).small().weak());
                                    } else {
                                        ui.label(RichText::new("no state").small().weak());
                                    }
                                    ui.horizontal(|ui| {
                                        let name = info.name.clone();
                                        if ui.button("Save").clicked() {
                                            action =
                                                Some((SlotAction::Save, info.slot, name));
                                        }
                                        if ui
                                            .add_enabled(
                                                info.exists,
                                                egui::Button::new("Load"),
                                            )
                                            .clicked()
                                        {
                                            action = Some((
                                                SlotAction::Load,
                                                info.slot,
                                                String::new(),
                                            ));
                                        }
                                        if ui
                                            .add_enabled(
                                                info.exists,
                                                egui::Button::new("Delete"),
                                            )
                                            .clicked()
                                        {
                                            action = Some((
                                                SlotAction::Delete,
                                                info.slot,
                                                String::new(),
                                            ));
                                        }
                                    });
                                });
                            });
                            ui.separator();
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.button("Prune orphaned files").clicked() {
                        action = Some((SlotAction::Prune, 0, String::new()));
                    }
                    if ui.button("Delete all slots").clicked() {
                        clear_confirm = true;
                    }
                    if clear_confirm {
                        if ui.button("Confirm delete all").clicked() {
                            action = Some((SlotAction::ClearAll, 0, String::new()));
                            clear_confirm = false;
                        }
                        if ui.button("Cancel").clicked() {
                            clear_confirm = false;
                        }
                    }
                    let used = infos.iter().filter(|info| info.exists).count();
                    ui.label(
                        RichText::new(format!("{used} of {SLOT_COUNT} slots used"))
                            .small()
                            .weak(),
                    );
                });
            });
        self.slot_manager_open = open;
        self.slot_thumbnails = textures;
        let reset_textures = matches!(action, Some((SlotAction::Save | SlotAction::Delete, _, _)));
        if let Some(player) = &mut self.player {
            for (slot, name) in renames {
                player.rename_slot(slot, &name);
            }
            if let Some((action, slot, name)) = action {
                match action {
                    SlotAction::Save => player.save_state_named(slot, &name),
                    SlotAction::Load => player.load_state(slot),
                    SlotAction::Delete => player.delete_state(slot),
                    SlotAction::Prune => player.prune_slots(),
                    SlotAction::ClearAll => player.clear_slots(),
                }
            }
        }
        if reset_textures {
            self.slot_thumbnails.clear();
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        egui::Window::new("About Unko")
            .open(&mut open)
            .default_size([460.0, 300.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Unko");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label("A multi-system, multi-core emulator shell.");
                ui.label("MIT licensed. NES backend: Unko's accuracy core.");
                ui.add_space(6.0);
                ui.label("Frontend structure inspired by Dolphin's desktop UX;");
                ui.label("no Dolphin code, icons or assets are used.");
                ui.add_space(6.0);
                ui.label("No ROMs are included.");
            });
        self.about_open = open;
    }

    fn open_rom_window(&mut self, ctx: &egui::Context) {
        if !self.open_rom_open {
            return;
        }
        let mut open = self.open_rom_open;
        let mut close = false;
        egui::Window::new("Open ROM")
            .open(&mut open)
            .default_size([480.0, 180.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Path to a ROM or archive (.nes, .zip, .7z, .rar):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.open_rom_text)
                        .desired_width(420.0)
                        .hint_text("/path/to/game.nes, .fds, .zip, .7z or .rar"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        let path = PathBuf::from(self.open_rom_text.trim());
                        if path.is_file() {
                            self.start_game(&path);
                            close = true;
                        } else {
                            self.set_status(format!("Not a file: {}", path.display()));
                        }
                    }
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM or archive", &["nes", "fds", "zip", "7z", "rar"])
                            .pick_file()
                        {
                            self.open_rom_text = path.display().to_string();
                            self.start_game(&path);
                            close = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
                ui.label(
                    RichText::new("Native file dialogs land when the desktop portal is available.")
                        .weak()
                        .small(),
                );
            });
        if close {
            open = false;
        }
        self.open_rom_open = open;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let play_now = Instant::now();
        let play_dt = self.play_tick.map_or(Duration::ZERO, |last| {
            play_now
                .duration_since(last)
                .min(Duration::from_millis(250))
        });
        self.play_tick = Some(play_now);
        self.rom_manager.poll();
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.handle_drop(dropped);
        }
        self.shortcuts(ctx);
        self.menu(ctx);
        self.toolbar(ctx);
        if self.settings.show_status_bar {
            self.status_bar(ctx);
        }

        // Save power when the window is inactive, without overwriting the
        // user's own pause state.
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        if self.settings.pause_on_focus_loss && !focused {
            if let Some(player) = &mut self.player {
                if !player.paused {
                    player.toggle_pause();
                    self.paused_by_focus = true;
                }
            }
        } else if self.paused_by_focus {
            if let Some(player) = &mut self.player {
                if player.paused {
                    player.toggle_pause();
                }
            }
            self.paused_by_focus = false;
        }

        let inputs = [self.settings.port1_input, self.settings.port2_input];
        let polled = if self.settings.gamepad_enabled {
            self.gamepads
                .as_mut()
                .map_or([unko_core::InputState::empty(); 4], |gamepads| {
                    gamepads.poll(
                        self.active_mapping,
                        self.settings.pad_deadzone,
                        self.settings.turbo_enabled,
                        self.settings.turbo_hz,
                    )
                })
        } else {
            [unko_core::InputState::empty(); 4]
        };
        let ui_pad = self
            .gamepads
            .as_ref()
            .and_then(|gamepads| gamepads.names().iter().position(Option::is_some))
            .map_or(unko_core::InputState::empty(), |slot| polled[slot]);
        self.handle_controller_ui(ctx, ui_pad);
        let mut routed = [unko_core::InputState::empty(); 4];
        let mut keyboard_ports = [false; 4];
        let mut explicit_slots = [false; 4];
        for (port, input) in inputs.iter().enumerate() {
            match input {
                PortInput::Auto => keyboard_ports[port] = port == 0,
                PortInput::None
                | PortInput::MouseZapper
                | PortInput::MousePaddle
                | PortInput::Mouse => {}
                PortInput::Keyboard => keyboard_ports[port] = true,
                PortInput::Gamepad(slot) => {
                    let slot = (*slot).min(3);
                    explicit_slots[slot] = true;
                    if !self.settings.pad_disabled[slot] {
                        routed[port] = unko_core::InputState::from_bits(
                            routed[port].bits() | polled[slot].bits(),
                        );
                    }
                }
            }
        }
        // Auto keeps the legacy gamepad matrix; explicit port choices win.
        for (slot, state) in polled.iter().enumerate() {
            if self.settings.pad_disabled[slot] || explicit_slots[slot] {
                continue;
            }
            let port = usize::from(self.settings.pad_ports[slot].min(3));
            if inputs
                .get(port)
                .is_some_and(|input| *input == PortInput::Auto)
            {
                routed[port] = unko_core::InputState::from_bits(routed[port].bits() | state.bits());
            }
        }
        self.input_states = routed;
        let mut enabled = [true; 4];
        for port in 0..2 {
            enabled[port] = !self.settings.port_disabled[port] && inputs[port] != PortInput::None;
            enabled[port + 2] = enabled[port];
        }
        if let Some(audio) = &self.audio {
            let running = self.player.as_ref().is_some_and(|player| {
                !player.paused && player.speed_percent() == 100 && !player.fast_forward()
            });
            audio.set_active(running && !self.settings.muted);
        }
        let detached_windows = self.detached_windows();
        self.poll_hole_punch();
        let window_generation = self.window_generation;
        let audio = self.audio.as_mut();
        if let Some(player) = &mut self.player {
            for (port, state) in self.input_states.iter().enumerate() {
                player.set_gamepad(port, *state);
            }
            player.set_keyboard_ports(keyboard_ports);
            // The two physical ports gate players 1/2 and, for a Four Score,
            // the extra players 3/4 that ride the same ports.
            for (port, enabled) in enabled.iter().enumerate() {
                player.set_port_enabled(port, *enabled);
            }
            let microphone = self
                .microphone_input
                .as_ref()
                .map_or(self.microphone, MicrophoneInput::active);
            player.set_microphone(microphone);
            player.step(ctx, audio);
        }

        // Count wall-clock time only while a game is actually advancing.
        let running_stem = self
            .player
            .as_ref()
            .filter(|player| !player.paused)
            .and_then(|_| self.play_stem.clone());
        if let Some(stem) = running_stem {
            self.play_pending += play_dt;
            let whole = self.play_pending.as_secs();
            if whole > 0 {
                self.play_pending -= Duration::from_secs(whole);
                self.play_stats.add_seconds(&stem, whole);
            }
        }
        if self.play_flush.elapsed() >= Duration::from_secs(30) {
            self.play_flush = Instant::now();
            self.flush_play_stats();
        }

        if let Some(player) = &mut self.player {
            let port1 = self.game_props.port1.unwrap_or(self.settings.port1_device);
            let port2 = self.game_props.port2.unwrap_or(self.settings.port2_device);
            // The Four Score adapter occupies both ports.
            let (device1, device2) =
                if port1 == PortDevice::FourScore || port2 == PortDevice::FourScore {
                    (PortDevice::FourScore, PortDevice::FourScore)
                } else {
                    (port1, port2)
                };
            player.set_port_devices(device1, device2);
            let zapper_port = if enabled[0] && port1 == PortDevice::Zapper {
                Some(0)
            } else if enabled[1] && port2 == PortDevice::Zapper {
                Some(1)
            } else {
                None
            };
            let paddle_port = if enabled[0] && port1 == PortDevice::Paddle {
                Some(0)
            } else if enabled[1] && port2 == PortDevice::Paddle {
                Some(1)
            } else {
                None
            };
            let mouse_port = if enabled[0] && port1 == PortDevice::Mouse {
                Some(0)
            } else if enabled[1] && port2 == PortDevice::Mouse {
                Some(1)
            } else {
                None
            };
            let power_pad_port = if enabled[0]
                && (port1 == PortDevice::PowerPad || port1 == PortDevice::FamilyTrainer)
            {
                Some(0)
            } else if enabled[1]
                && (port2 == PortDevice::PowerPad || port2 == PortDevice::FamilyTrainer)
            {
                Some(1)
            } else {
                None
            };
            if enabled[0] && port1 == PortDevice::FamilyKeyboard
                || enabled[1] && port2 == PortDevice::FamilyKeyboard
            {
                player.set_family_keyboard(family_keyboard_matrix(ctx));
            }
            if enabled[0] && port1 == PortDevice::SuborKeyboard
                || enabled[1] && port2 == PortDevice::SuborKeyboard
            {
                player.set_subor_keyboard(subor_keyboard_matrix(ctx));
            }
            let view = viewport(
                ctx,
                player,
                &ViewportOptions {
                    integer_scale: self.settings.integer_scale,
                    linear_filter: self.settings.linear_filter,
                    aspect: self.settings.aspect,
                    scale: self.settings.scale,
                    supersample: self.settings.supersample,
                    shader: self.settings.shader,
                    brightness: self.settings.brightness,
                    sharpness: self.settings.sharpness,
                    overscan: self.settings.overscan,
                    mask_edges: self.settings.mask_edges,
                    zapper_port,
                    paddle_port,
                    mouse_port,
                    power_pad_port,
                    input: self.input_states,
                    show_input: self.settings.show_input_display,
                    show_touch: self.settings.show_touch_controls,
                },
                #[cfg(not(target_os = "ios"))]
                self.gpu_shader.as_ref(),
            );
            player.set_touch(view.touch);
            if let (Some(port), Some((position, fire))) = (paddle_port, view.paddle) {
                player.set_paddle(port, position, fire);
            }
            if let (Some(port), Some((dx, dy, left, right))) = (mouse_port, view.mouse) {
                player.set_mouse(port, dx, dy, left, right);
            }
            if let (Some(port), Some(mask)) = (power_pad_port, view.power_pad) {
                player.set_power_pad(port, mask);
            }
            if player.debug.open {
                if let Some(view) = player.debug_view() {
                    let mut layout = self.settings.layout_debugger;
                    let actions = debugger::window(
                        &mut player.debug,
                        ctx,
                        &view,
                        detached_windows,
                        window_generation,
                        &mut layout,
                        &mut self.debugger_window_shown,
                    );
                    self.settings.layout_debugger = layout;
                    for action in actions {
                        player.apply_debug(action, ctx);
                    }
                }
            }
        } else {
            self.library(ctx);
        }

        let desired_title = match &self.player {
            Some(player) => format!("Unko — {}", player.title),
            None => "Unko".to_string(),
        };
        if desired_title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(desired_title.clone()));
            self.window_title = desired_title;
        }

        self.settings_window(ctx);
        self.devices_window(ctx);
        self.cheats_window(ctx);
        self.game_properties_window(ctx);
        self.slot_window(ctx);
        self.netplay_window(ctx);
        self.rom_manager_window(ctx);
        self.controller_menu(ctx);
        self.about_window(ctx);
        self.open_rom_window(ctx);
        self.reset_confirmation(ctx);

        if let Some(entry) = self.pending_play.take() {
            self.start_entry(entry);
        }

        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let info = ctx.input(|input| input.viewport().clone());
            if let Some(outer) = info.outer_rect {
                let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                self.settings.layout_main = Some([outer.min.x, outer.min.y, size.x, size.y]);
            }
        }

        if let Some(player) = &self.player {
            ctx.request_repaint_after(player.next_repaint());
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.shutdown();
        if let (Some(gl), Some(shader)) = (gl, &self.gpu_shader) {
            if let Ok(mut shader) = shader.lock() {
                shader.destroy(gl);
            }
        }
    }

    #[cfg(target_os = "ios")]
    fn on_exit(&mut self) {
        self.shutdown();
    }
}

impl App {
    /// Stops the audio stream and persists settings when the shell exits.
    fn shutdown(&mut self) {
        self.flush_play_stats();
        self.persist_vs_config();
        if let Some(player) = &mut self.player {
            player.shutdown();
        }
        let _ = self.settings.save(&Settings::path());
    }
}

fn scan(dir: &std::path::Path) -> Vec<GameEntry> {
    if dir.is_dir() {
        library::scan(dir)
    } else {
        Vec::new()
    }
}

fn resolve_first(path: &Path) -> Option<GameEntry> {
    library::probe_path(path).ok()?.into_iter().next()
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_visuals(match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    });
}

fn modified_label(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "?".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_secs());
    let ago = now.saturating_sub(seconds);
    if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else if ago < 86_400 {
        format!("{}h ago", ago / 3600)
    } else {
        format!("{}d ago", ago / 86_400)
    }
}

fn fit_aspect(available: Rect, aspect: f32) -> Rect {
    let size = available.size();
    let (width, height) = if size.x / size.y > aspect {
        (size.y * aspect, size.y)
    } else {
        (size.x, size.x / aspect)
    };
    Rect::from_center_size(available.center(), Vec2::new(width, height))
}

/// Masks the 8-pixel software-mask columns that analog TVs commonly cropped.
fn paint_mask(ui: &egui::Ui, target: Rect, mask: MaskEdges) {
    let width = target.width() * 8.0 / 256.0;
    let black = Color32::BLACK;
    if matches!(mask, MaskEdges::Left | MaskEdges::Both | MaskEdges::Auto) {
        ui.painter().rect_filled(
            Rect::from_min_max(
                target.left_top(),
                egui::pos2(target.left() + width, target.bottom()),
            ),
            0.0,
            black,
        );
    }
    if mask == MaskEdges::Both {
        ui.painter().rect_filled(
            Rect::from_min_max(
                egui::pos2(target.right() - width, target.top()),
                target.right_bottom(),
            ),
            0.0,
            black,
        );
    }
}

/// Draws the selected output post-process preset over the game image.
fn paint_shader(ui: &egui::Ui, target: Rect, shader: ShaderPreset) {
    let step = (target.height() / 240.0).max(1.0);
    match shader {
        ShaderPreset::Off => {}
        ShaderPreset::Scanlines | ShaderPreset::Crt => {
            let painter = ui.painter();
            let mut y = target.top();
            let mut row = 0;
            while y < target.bottom() {
                if row & 1 == 1 {
                    let line = Rect::from_min_size(
                        egui::pos2(target.left(), y),
                        Vec2::new(target.width(), (step * 0.45).max(1.0)),
                    );
                    painter.rect_filled(
                        line,
                        0.0,
                        Color32::from_black_alpha(if shader == ShaderPreset::Crt {
                            90
                        } else {
                            64
                        }),
                    );
                }
                y += step;
                row += 1;
            }
            if shader == ShaderPreset::Crt {
                // Cheap vignette: two nested borders instead of a shader.
                let border = (target.width().min(target.height()) * 0.06).max(6.0);
                painter.rect_stroke(
                    target.shrink(border * 0.5),
                    6.0,
                    Stroke::new(border, Color32::from_black_alpha(60)),
                    egui::StrokeKind::Inside,
                );
                painter.rect_stroke(
                    target,
                    6.0,
                    Stroke::new(border * 2.0, Color32::from_black_alpha(35)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ShaderPreset::NtscBudget | ShaderPreset::NtscPremium => {}
        ShaderPreset::Lcd => {
            let painter = ui.painter();
            let color = Color32::from_black_alpha(48);
            let mut x = target.left();
            while x <= target.right() {
                painter.line_segment(
                    [egui::pos2(x, target.top()), egui::pos2(x, target.bottom())],
                    Stroke::new(1.0_f32, color),
                );
                x += step;
            }
            let mut y = target.top();
            while y <= target.bottom() {
                painter.line_segment(
                    [egui::pos2(target.left(), y), egui::pos2(target.right(), y)],
                    Stroke::new(1.0_f32, color),
                );
                y += step;
            }
        }
    }
}

struct ViewportOptions {
    integer_scale: bool,
    linear_filter: bool,
    aspect: Aspect,
    scale: u32,
    supersample: u32,
    shader: ShaderPreset,
    brightness: f32,
    sharpness: f32,
    overscan: Overscan,
    mask_edges: MaskEdges,
    zapper_port: Option<usize>,
    paddle_port: Option<usize>,
    mouse_port: Option<usize>,
    power_pad_port: Option<usize>,
    input: [unko_core::InputState; 4],
    show_input: bool,
    show_touch: bool,
}

/// Input the viewport produced this frame (touch, paddle or mat).
struct ViewportInput {
    touch: unko_core::InputState,
    paddle: Option<(u8, bool)>,
    /// Mouse movement in mickeys (right/up positive) and its two buttons.
    mouse: Option<(i32, i32, bool, bool)>,
    power_pad: Option<u16>,
}

fn viewport(
    ctx: &egui::Context,
    player: &mut Player,
    options: &ViewportOptions,
    #[cfg(not(target_os = "ios"))] gpu: Option<&crate::gpu_shader::Handle>,
) -> ViewportInput {
    let ViewportOptions {
        integer_scale,
        linear_filter,
        aspect,
        scale,
        supersample,
        shader,
        brightness,
        sharpness,
        overscan,
        mask_edges,
        zapper_port,
        paddle_port,
        mouse_port,
        power_pad_port,
        input,
        show_input,
        show_touch,
    } = *options;
    let mut touch = unko_core::InputState::empty();
    #[cfg(target_os = "ios")]
    let _ = (brightness, sharpness);
    let mut paddle = None;
    let mut mouse = None;
    let mut power_pad = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ctx, |ui| {
            let available = ui.max_rect();
            let mut target = match aspect.ratio() {
                Some(ratio) => fit_aspect(available, ratio),
                None => available,
            };
            if scale > 0 {
                let factor = scale as f32;
                let size = Vec2::new(256.0 * factor, 240.0 * factor);
                if size.x <= available.width() && size.y <= available.height() {
                    target = Rect::from_center_size(available.center(), size);
                }
            } else if integer_scale {
                let factor = (target.height() / 240.0).floor().max(1.0);
                let size = Vec2::new(240.0 * factor * 4.0 / 3.0, 240.0 * factor);
                target = Rect::from_center_size(available.center(), size);
            }
            if let Some(port) = zapper_port {
                // Mouse aims and fires the Zapper. The pointer is mapped into
                // the 256x240 frame inside the scaled viewport rect; leaving
                // the rect keeps the trigger released.
                let position = ctx.pointer_latest_pos();
                let trigger = ctx.input(|input| input.pointer.primary_down());
                if let Some(position) = position.filter(|position| target.contains(*position)) {
                    let relative = (position - target.min) / target.size();
                    let x = (relative.x * 256.0).clamp(0.0, 255.0) as u8;
                    let y = (relative.y * 240.0).clamp(0.0, 239.0) as u8;
                    player.set_zapper(port, x, y, trigger);
                } else if trigger {
                    player.set_zapper(port, 0, 0, false);
                }
            }
            if paddle_port.is_some() {
                // The paddle knob follows the horizontal mouse position (or
                // the touch point); the primary button fires.
                let position = ctx.pointer_latest_pos();
                let down = ctx.input(|input| input.pointer.primary_down());
                paddle = Some(
                    match position.filter(|position| target.contains(*position)) {
                        Some(position) => {
                            let relative = (position.x - target.min.x) / target.width();
                            ((relative * 255.0).clamp(0.0, 255.0) as u8, down)
                        }
                        None => (128, false),
                    },
                );
            }
            if mouse_port.is_some() {
                // The mouse reports movement since the previous report, so use
                // egui's per-frame pointer delta rather than an absolute
                // position. Movement and clicks outside the viewport are
                // ignored so the surrounding UI stays usable.
                let inside = ctx
                    .pointer_latest_pos()
                    .is_some_and(|position| target.contains(position));
                let (delta, buttons) = ctx.input(|input| {
                    (
                        input.pointer.delta(),
                        (input.pointer.primary_down(), input.pointer.secondary_down()),
                    )
                });
                mouse = Some(if inside {
                    (
                        delta.x.round() as i32,
                        -delta.y.round() as i32,
                        buttons.0,
                        buttons.1,
                    )
                } else {
                    (0, 0, false, false)
                });
            }
            player.set_texture_options(if linear_filter || supersample > 1 {
                egui::TextureOptions::LINEAR
            } else {
                egui::TextureOptions::NEAREST
            });
            if let Some(texture) = player.texture() {
                let id = texture.id();
                let (vx, vy, vw, vh) = overscan.visible();
                let uv = Rect::from_min_max(
                    egui::pos2(f32::from(vx) / 256.0, f32::from(vy) / 240.0),
                    egui::pos2(f32::from(vx + vw) / 256.0, f32::from(vy + vh) / 240.0),
                );
                #[cfg(not(target_os = "ios"))]
                let gpu_drawn = if shader.is_gpu() {
                    match (gpu, player.frame_rgba()) {
                        (Some(gpu), Some((pixels, width, height))) => {
                            crate::gpu_shader::GpuShader::queue(
                                gpu,
                                ui,
                                target,
                                uv,
                                crate::gpu_shader::ShaderParams {
                                    preset: shader.shader_index(),
                                    brightness,
                                    sharpness,
                                    linear: linear_filter || supersample > 1,
                                },
                                pixels,
                                width,
                                height,
                            );
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                #[cfg(target_os = "ios")]
                let gpu_drawn = false;
                if !gpu_drawn {
                    ui.painter().image(id, target, uv, Color32::WHITE);
                    paint_shader(ui, target, shader);
                }
                paint_mask(ui, target, mask_edges);
            } else {
                ui.painter().text(
                    available.center(),
                    Align2::CENTER_CENTER,
                    "Starting...",
                    FontId::proportional(18.0),
                    Color32::GRAY,
                );
            }

            if player.paused {
                let text = "Paused — Space or Pause to resume";
                let galley = ui.painter().layout_no_wrap(
                    text.into(),
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
                let bg = Rect::from_center_size(
                    available.center(),
                    galley.size() + Vec2::new(24.0, 12.0),
                );
                ui.painter().rect_filled(
                    bg,
                    egui::CornerRadius::same(4),
                    Color32::from_black_alpha(180),
                );
                ui.painter()
                    .galley(bg.center() - galley.size() / 2.0, galley, Color32::WHITE);
            }

            if !player.message.is_empty() {
                ui.painter().text(
                    available.left_bottom() + Vec2::new(12.0, -12.0),
                    Align2::LEFT_BOTTOM,
                    &player.message,
                    FontId::proportional(13.0),
                    Color32::from_gray(200),
                );
            }

            if show_input {
                let text = format!(
                    "P1 {}   P2 {}",
                    input_label(input[0]),
                    input_label(input[1])
                );
                let galley =
                    ui.painter()
                        .layout_no_wrap(text, FontId::monospace(13.0), Color32::WHITE);
                let pos = target.left_top() + Vec2::new(10.0, 10.0);
                let bg = Rect::from_min_size(pos, galley.size() + Vec2::new(12.0, 8.0));
                ui.painter().rect_filled(
                    bg,
                    egui::CornerRadius::same(4),
                    Color32::from_black_alpha(160),
                );
                ui.painter()
                    .galley(pos + Vec2::new(6.0, 4.0), galley, Color32::WHITE);
            }

            if show_touch {
                let pointer = ctx.pointer_latest_pos();
                let down = ctx.input(|i| i.pointer.primary_down());
                for (button, rect, label) in touch_buttons(target) {
                    let pressed = down && pointer.is_some_and(|pos| rect.contains(pos));
                    if pressed {
                        touch = touch.with(button, true);
                    }
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(8),
                        if pressed {
                            Color32::from_white_alpha(150)
                        } else {
                            Color32::from_white_alpha(55)
                        },
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            }

            if power_pad_port.is_some() {
                // Twelve-button mat: 3 rows x 4 columns, numbered row-major.
                let pointer = ctx.pointer_latest_pos();
                let down = ctx.input(|i| i.pointer.primary_down());
                let cell = (target.width().min(target.height()) * 0.075).clamp(30.0, 58.0);
                let grid = Vec2::new(cell * 4.0, cell * 3.0);
                let origin =
                    target.center_bottom() + Vec2::new(-grid.x / 2.0, -grid.y - cell * 0.6);
                let mut mask = 0u16;
                for row in 0..3 {
                    for column in 0..4 {
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(column as f32 * cell, row as f32 * cell),
                            Vec2::splat(cell - 5.0),
                        );
                        let number = row * 4 + column + 1;
                        let pressed = down && pointer.is_some_and(|pos| rect.contains(pos));
                        if pressed {
                            mask |= 1 << (number - 1);
                        }
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(6),
                            if pressed {
                                Color32::from_white_alpha(170)
                            } else {
                                Color32::from_white_alpha(60)
                            },
                        );
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            number.to_string(),
                            FontId::proportional(15.0),
                            Color32::WHITE,
                        );
                    }
                }
                power_pad = Some(mask);
            }
        });
    ViewportInput {
        touch,
        paddle,
        mouse,
        power_pad,
    }
}

/// Builds the Family BASIC keyboard matrix from the host keyboard.
///
/// The 9x8 matrix follows the NESdev wiki layout: each row has two column
/// halves of four keys, selected by `$4016` bit 0. Keys that do not exist on a
/// US layout (STOP, kana, GRPH, RSHIFT) are not mapped; Backslash types the
/// yen key, which is its documented historical equivalent.
/// Host-key map for the Subor keyboard matrix. A position is 0-3 for the
/// lower nibble and 4-7 for the upper nibble of each row.
const SUBOR_MAP: &[(egui::Key, usize, u8)] = &[
    (egui::Key::Num4, 0, 0),
    (egui::Key::G, 0, 1),
    (egui::Key::F, 0, 2),
    (egui::Key::C, 0, 3),
    (egui::Key::F2, 0, 4),
    (egui::Key::E, 0, 5),
    (egui::Key::Num5, 0, 6),
    (egui::Key::V, 0, 7),
    (egui::Key::Num2, 1, 0),
    (egui::Key::D, 1, 1),
    (egui::Key::S, 1, 2),
    (egui::Key::End, 1, 3),
    (egui::Key::F1, 1, 4),
    (egui::Key::W, 1, 5),
    (egui::Key::Num3, 1, 6),
    (egui::Key::X, 1, 7),
    (egui::Key::Insert, 2, 0),
    (egui::Key::Backspace, 2, 1),
    (egui::Key::PageDown, 2, 2),
    (egui::Key::ArrowRight, 2, 3),
    (egui::Key::F8, 2, 4),
    (egui::Key::PageUp, 2, 5),
    (egui::Key::Delete, 2, 6),
    (egui::Key::Home, 2, 7),
    (egui::Key::Num9, 3, 0),
    (egui::Key::I, 3, 1),
    (egui::Key::L, 3, 2),
    (egui::Key::Comma, 3, 3),
    (egui::Key::F5, 3, 4),
    (egui::Key::O, 3, 5),
    (egui::Key::Num0, 3, 6),
    (egui::Key::Period, 3, 7),
    (egui::Key::CloseBracket, 4, 0),
    (egui::Key::Enter, 4, 1),
    (egui::Key::ArrowUp, 4, 2),
    (egui::Key::ArrowLeft, 4, 3),
    (egui::Key::F7, 4, 4),
    (egui::Key::OpenBracket, 4, 5),
    (egui::Key::Backslash, 4, 6),
    (egui::Key::ArrowDown, 4, 7),
    (egui::Key::Q, 5, 0),
    (egui::Key::Z, 5, 2),
    (egui::Key::Tab, 5, 3),
    (egui::Key::Escape, 5, 4),
    (egui::Key::A, 5, 5),
    (egui::Key::Num1, 5, 6),
    (egui::Key::Num7, 6, 0),
    (egui::Key::Y, 6, 1),
    (egui::Key::K, 6, 2),
    (egui::Key::M, 6, 3),
    (egui::Key::F4, 6, 4),
    (egui::Key::U, 6, 5),
    (egui::Key::Num8, 6, 6),
    (egui::Key::J, 6, 7),
    (egui::Key::Minus, 7, 0),
    (egui::Key::Semicolon, 7, 1),
    (egui::Key::Quote, 7, 2),
    (egui::Key::Slash, 7, 3),
    (egui::Key::F6, 7, 4),
    (egui::Key::P, 7, 5),
    (egui::Key::Equals, 7, 6),
    (egui::Key::T, 8, 0),
    (egui::Key::H, 8, 1),
    (egui::Key::N, 8, 2),
    (egui::Key::Space, 8, 3),
    (egui::Key::F3, 8, 4),
    (egui::Key::R, 8, 5),
    (egui::Key::Num6, 8, 6),
    (egui::Key::B, 8, 7),
    (egui::Key::F11, 10, 3),
    (egui::Key::F12, 10, 4),
    (egui::Key::F10, 11, 4),
    (egui::Key::Backtick, 12, 0),
    (egui::Key::F9, 12, 4),
];

fn subor_keyboard_matrix(ctx: &egui::Context) -> [u8; 13] {
    let mut rows = [0u8; 13];
    ctx.input(|input| {
        for &(key, row, bit) in SUBOR_MAP {
            if input.key_down(key) {
                rows[row] |= 1 << bit;
            }
        }
        // The matrix's Shift and Ctrl cells are modifiers, not key codes.
        if input.modifiers.shift {
            rows[7] |= 1 << 7;
        }
        if input.modifiers.ctrl {
            rows[5] |= 1 << 7;
        }
    });
    rows
}

fn family_keyboard_matrix(ctx: &egui::Context) -> [u8; 9] {
    // (host key, row, column half, position within the half)
    const MAP: &[(egui::Key, u8, u8, u8)] = &[
        (egui::Key::CloseBracket, 0, 0, 0),
        (egui::Key::OpenBracket, 0, 0, 1),
        (egui::Key::Enter, 0, 0, 2),
        (egui::Key::F8, 0, 0, 3),
        (egui::Key::Backslash, 0, 1, 1),
        (egui::Key::F7, 1, 0, 3),
        (egui::Key::Slash, 1, 1, 2),
        (egui::Key::K, 2, 0, 0),
        (egui::Key::L, 2, 0, 1),
        (egui::Key::O, 2, 0, 2),
        (egui::Key::F6, 2, 0, 3),
        (egui::Key::Num0, 2, 1, 0),
        (egui::Key::P, 2, 1, 1),
        (egui::Key::Comma, 2, 1, 2),
        (egui::Key::Period, 2, 1, 3),
        (egui::Key::J, 3, 0, 0),
        (egui::Key::U, 3, 0, 1),
        (egui::Key::I, 3, 0, 2),
        (egui::Key::F5, 3, 0, 3),
        (egui::Key::Num8, 3, 1, 0),
        (egui::Key::Num9, 3, 1, 1),
        (egui::Key::N, 3, 1, 2),
        (egui::Key::M, 3, 1, 3),
        (egui::Key::H, 4, 0, 0),
        (egui::Key::G, 4, 0, 1),
        (egui::Key::Y, 4, 0, 2),
        (egui::Key::F4, 4, 0, 3),
        (egui::Key::Num7, 4, 1, 1),
        (egui::Key::V, 4, 1, 2),
        (egui::Key::B, 4, 1, 3),
        (egui::Key::D, 5, 0, 0),
        (egui::Key::R, 5, 0, 1),
        (egui::Key::T, 5, 0, 2),
        (egui::Key::F3, 5, 0, 3),
        (egui::Key::Num4, 5, 1, 0),
        (egui::Key::Num5, 5, 1, 1),
        (egui::Key::C, 5, 1, 2),
        (egui::Key::F, 5, 1, 3),
        (egui::Key::A, 6, 0, 0),
        (egui::Key::S, 6, 0, 1),
        (egui::Key::W, 6, 0, 2),
        (egui::Key::F2, 6, 0, 3),
        (egui::Key::Num3, 6, 1, 0),
        (egui::Key::E, 6, 1, 1),
        (egui::Key::Z, 6, 1, 2),
        (egui::Key::X, 6, 1, 3),
        (egui::Key::Q, 7, 0, 1),
        (egui::Key::Escape, 7, 0, 2),
        (egui::Key::F1, 7, 0, 3),
        (egui::Key::Num1, 7, 1, 1),
        (egui::Key::ArrowLeft, 8, 0, 0),
        (egui::Key::ArrowRight, 8, 0, 1),
        (egui::Key::ArrowUp, 8, 0, 2),
        (egui::Key::Home, 8, 0, 3),
        (egui::Key::Insert, 8, 1, 0),
        (egui::Key::Backspace, 8, 1, 1),
        (egui::Key::Space, 8, 1, 2),
        (egui::Key::ArrowDown, 8, 1, 3),
    ];
    let mut keys = [0u8; 9];
    ctx.input(|input| {
        let bit = |half: u8, position: u8| -> u8 {
            if half == 0 {
                3 - position
            } else {
                4 + (3 - position)
            }
        };
        for (key, row, half, position) in MAP {
            if input.key_down(*key) {
                keys[usize::from(*row)] |= 1 << bit(*half, *position);
            }
        }
        if input.modifiers.shift {
            keys[7] |= 1 << bit(1, 3); // LSHIFT
        }
        if input.modifiers.ctrl {
            keys[7] |= 1 << bit(0, 0); // CTR
        }
        // Shifted punctuation that lives on another matrix key.
        // Keys whose shifted symbol is a separate matrix key: press the
        // symbol that matches the shift state instead of both switches.
        let shift = input.modifiers.shift;
        if input.key_down(egui::Key::Semicolon) {
            keys[1] |= 1 << if shift { bit(0, 1) } else { bit(0, 0) };
        }
        if input.key_down(egui::Key::Minus) {
            keys[1] |= 1 << if shift { bit(1, 3) } else { bit(1, 1) };
        }
        if input.key_down(egui::Key::Num2) {
            if shift {
                keys[1] |= 1 << bit(0, 2); // @
            } else {
                keys[7] |= 1 << bit(1, 0); // 2
            }
        }
        if input.key_down(egui::Key::Num6) {
            if shift {
                keys[1] |= 1 << bit(1, 0); // ^
            } else {
                keys[4] |= 1 << bit(1, 0); // 6
            }
        }
    });
    keys
}

fn input_label(state: unko_core::InputState) -> String {
    let mut parts = Vec::new();
    for (bit, label) in [
        (unko_core::InputState::UP, "↑"),
        (unko_core::InputState::DOWN, "↓"),
        (unko_core::InputState::LEFT, "←"),
        (unko_core::InputState::RIGHT, "→"),
        (unko_core::InputState::A, "A"),
        (unko_core::InputState::B, "B"),
        (unko_core::InputState::START, "Start"),
        (unko_core::InputState::SELECT, "Select"),
    ] {
        if state.bits() & bit != 0 {
            parts.push(label);
        }
    }
    if parts.is_empty() {
        "—".into()
    } else {
        parts.join(" ")
    }
}

fn touch_buttons(target: Rect) -> [(u16, Rect, &'static str); 8] {
    let cell = (target.width().min(target.height()) * 0.075).clamp(28.0, 54.0);
    let pad = target.left_bottom() + Vec2::new(cell * 1.4, -cell * 2.8);
    let dpad = |dx: f32, dy: f32| {
        Rect::from_center_size(pad + Vec2::new(dx * cell, dy * cell), Vec2::splat(cell))
    };
    let right = target.right_bottom();
    let a = Rect::from_center_size(
        right + Vec2::new(-cell * 2.2, -cell * 2.4),
        Vec2::splat(cell),
    );
    let b = Rect::from_center_size(
        right + Vec2::new(-cell * 3.5, -cell * 1.5),
        Vec2::splat(cell),
    );
    let start = Rect::from_center_size(
        right + Vec2::new(-cell * 1.0, -cell * 0.5),
        Vec2::new(cell * 1.6, cell * 0.7),
    );
    let select = Rect::from_center_size(
        right + Vec2::new(-cell * 3.0, -cell * 0.5),
        Vec2::new(cell * 1.6, cell * 0.7),
    );
    [
        (unko_core::InputState::UP, dpad(0.0, -1.0), "↑"),
        (unko_core::InputState::DOWN, dpad(0.0, 1.0), "↓"),
        (unko_core::InputState::LEFT, dpad(-1.0, 0.0), "←"),
        (unko_core::InputState::RIGHT, dpad(1.0, 0.0), "→"),
        (unko_core::InputState::A, a, "A"),
        (unko_core::InputState::B, b, "B"),
        (unko_core::InputState::START, start, "Start"),
        (unko_core::InputState::SELECT, select, "Select"),
    ]
}

fn general_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    _ctx: &egui::Context,
    reset_layout: &mut bool,
    play: &PlayStats,
    reset_play: &mut bool,
) -> bool {
    ui.heading("General");
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.pause_on_focus_loss,
        "Pause when the window loses focus",
    );
    ui.checkbox(&mut settings.confirm_stop, "Confirm before stopping a game");
    ui.checkbox(
        &mut settings.detachable_windows,
        "Open Settings, Devices and the Debugger as separate OS windows",
    );
    if settings.detachable_windows && ui.button("Reset window layout").clicked() {
        *reset_layout = true;
    }
    ui.add_space(12.0);
    ui.label("Theme");
    ui.horizontal(|ui| {
        ui.radio_value(&mut settings.theme, Theme::Dark, "Dark");
        ui.radio_value(&mut settings.theme, Theme::Light, "Light");
    });
    ui.add_space(18.0);
    ui.separator();
    ui.label("Play time");
    let summary = if play.is_empty() {
        "No play time tracked yet.".to_string()
    } else {
        format!(
            "{} tracked across {} game(s)",
            format_duration(play.total_seconds()),
            play.len()
        )
    };
    ui.label(RichText::new(summary).weak().small());
    if ui.button("Clear play-time counters").clicked() {
        *reset_play = true;
    }
    ui.add_space(12.0);
    ui.separator();
    let mut reset = false;
    if ui.button("Reset all settings…").clicked() {
        reset = true;
    }
    ui.label(
        RichText::new(
            "Restores configuration, window layout, hotkeys, gamepad bindings, \
             per-port devices and per-game profiles. Save states, SRAM and \
             recordings are kept.",
        )
        .weak()
        .small(),
    );
    reset
}

fn interface_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Interface");
    ui.add_space(8.0);
    ui.checkbox(&mut settings.show_status_bar, "Show status bar");
    ui.checkbox(&mut settings.grid_view, "Game library opens in grid view");
    ui.horizontal(|ui| {
        ui.label("Playback speed");
        egui::ComboBox::from_id_salt("playback-speed")
            .selected_text(format!("{}%", settings.speed_percent))
            .show_ui(ui, |ui| {
                for percent in [25, 50, 100, 150, 200, 300] {
                    ui.selectable_value(
                        &mut settings.speed_percent,
                        percent,
                        format!("{percent}%"),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Rewind buffer");
        egui::ComboBox::from_id_salt("rewind-budget")
            .selected_text(format!("{} MiB", settings.rewind_mb))
            .show_ui(ui, |ui| {
                for budget in [64, 128, 256] {
                    ui.selectable_value(&mut settings.rewind_mb, budget, format!("{budget} MiB"));
                }
            });
    });
    ui.label(
        RichText::new(
            "Hold the fast-forward hotkey (Tab) to temporarily run at 300%; hold Backspace \
             while paused to rewind one frame at a time.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label("Toolbar, menu and status layout follow Dolphin's desktop shell.");
}

fn graphics_tab(ui: &mut egui::Ui, settings: &mut Settings, ctx: &egui::Context) {
    ui.heading("Graphics");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Window size");
        for (label, size) in [
            ("1280x960", [1280.0, 960.0]),
            ("1600x1200", [1600.0, 1200.0]),
            ("1920x1080", [1920.0, 1080.0]),
        ] {
            if ui.button(label).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::from(size)));
            }
        }
    });
    ui.add_space(8.0);
    ui.label("Video filter");
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Raw,
        "Raw pixels (nearest)",
    );
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Composite,
        "Composite / NTSC simulation",
    );
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Crt,
        "CRT (composite + scanlines + bloom)",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Palette");
        egui::ComboBox::from_id_salt("palette")
            .selected_text(palette_label(settings.palette))
            .show_ui(ui, |ui| {
                for palette in [
                    unko_core::Palette::Canonical,
                    unko_core::Palette::Consumer,
                    unko_core::Palette::Rgb,
                    unko_core::Palette::Monochrome,
                    unko_core::Palette::Vivid,
                    unko_core::Palette::Soft,
                    unko_core::Palette::Pal,
                    unko_core::Palette::Studio,
                    unko_core::Palette::Custom,
                ] {
                    ui.selectable_value(&mut settings.palette, palette, palette_label(palette));
                }
            });
        ui.label("Palette file");
        let mut path = settings.custom_palette.display().to_string();
        if ui
            .add(egui::TextEdit::singleline(&mut path).desired_width(320.0))
            .changed()
        {
            settings.custom_palette = PathBuf::from(path);
            settings.palette = unko_core::Palette::Custom;
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Import .pal…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("NES palette", &["pal", "palette"])
                .pick_file()
            {
                settings.custom_palette = path;
                settings.palette = unko_core::Palette::Custom;
            }
        }
    });
    ui.label(
        RichText::new(
            "Canonical matches the reference oracle; the other palettes are display presets.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label("Aspect");
    ui.horizontal(|ui| {
        for aspect in [Aspect::Fit, Aspect::Tv43, Aspect::Pixel87] {
            ui.radio_value(&mut settings.aspect, aspect, aspect.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Overscan");
        egui::ComboBox::from_id_salt("overscan")
            .selected_text(settings.overscan.label())
            .show_ui(ui, |ui| {
                for overscan in [
                    Overscan::Normal,
                    Overscan::Vertical,
                    Overscan::Borders,
                    Overscan::Everything,
                ] {
                    ui.selectable_value(&mut settings.overscan, overscan, overscan.label());
                }
            });
        ui.label("Mask edges");
        egui::ComboBox::from_id_salt("mask-edges")
            .selected_text(settings.mask_edges.label())
            .show_ui(ui, |ui| {
                for mask in [
                    MaskEdges::Off,
                    MaskEdges::Left,
                    MaskEdges::Both,
                    MaskEdges::Auto,
                ] {
                    ui.selectable_value(&mut settings.mask_edges, mask, mask.label());
                }
            });
    });
    ui.checkbox(&mut settings.integer_scale, "Integer scaling when possible");
    ui.add(egui::Slider::new(&mut settings.scale, 0..=6).text("Fixed integer scale (0 = auto)"));
    ui.horizontal(|ui| {
        ui.label("Supersampling");
        egui::ComboBox::from_id_salt("supersample")
            .selected_text(format!("{}x", settings.supersample))
            .show_ui(ui, |ui| {
                for factor in [1, 2, 3] {
                    ui.selectable_value(&mut settings.supersample, factor, format!("{factor}x"));
                }
            });
        ui.label("(output upscale, then linear downscale)");
    });
    ui.checkbox(
        &mut settings.linear_filter,
        "Smooth (linear) texture filtering",
    );
    ui.checkbox(
        &mut settings.famicom_3d,
        "Famicom 3D glasses (red/cyan anaglyph)",
    );
    ui.label(
        RichText::new(
            "3D games select the eye through $4016 bit 1 (OUT1). Consecutive left and \
             right fields are combined into a red/cyan anaglyph; without \
             shutter glasses the depth is approximate.",
        )
        .weak()
        .small(),
    );
    ui.horizontal(|ui| {
        ui.label("Output shader");
        egui::ComboBox::from_id_salt("output-shader")
            .selected_text(settings.shader.label())
            .show_ui(ui, |ui| {
                for shader in [
                    ShaderPreset::Off,
                    ShaderPreset::Scanlines,
                    ShaderPreset::Lcd,
                    ShaderPreset::Crt,
                    ShaderPreset::NtscBudget,
                    ShaderPreset::NtscPremium,
                ] {
                    ui.selectable_value(&mut settings.shader, shader, shader.label());
                }
            });
    });
    ui.add(
        egui::Slider::new(&mut settings.brightness, 0.5..=1.5)
            .text("Brightness")
            .fixed_decimals(2),
    );
    ui.add(
        egui::Slider::new(&mut settings.sharpness, 0.0..=1.0)
            .text("Sharpness")
            .fixed_decimals(2),
    );
    ui.label(
        RichText::new(
            "Scanline, LCD, CRT and NTSC presets run in a GPU fragment shader. \
             NTSC Budget uses a 3-tap chroma filter, Premium a 7-tap filter; \
             brightness and sharpness apply to every preset.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Filter, aspect, scale and shader changes apply to the running game immediately.",
        )
        .weak()
        .small(),
    );
}

fn system_label(system: &str) -> &'static str {
    match system {
        "snes" => "SNES (stub)",
        "n64" => "N64 (stub)",
        _ => "NES",
    }
}

fn palette_label(palette: unko_core::Palette) -> &'static str {
    match palette {
        unko_core::Palette::Canonical => "Canonical 2C02",
        unko_core::Palette::Consumer => "Consumer TV (warm)",
        unko_core::Palette::Rgb => "RGB monitor (saturated)",
        unko_core::Palette::Monochrome => "Monochrome",
        unko_core::Palette::Custom => "Custom (.pal)",
        unko_core::Palette::Vivid => "Vivid (generated)",
        unko_core::Palette::Soft => "Soft (generated)",
        unko_core::Palette::Pal => "PAL-like (generated)",
        unko_core::Palette::Studio => "Studio (generated)",
    }
}

fn audio_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    audio: Option<&AudioOutput>,
    audio_error: Option<&str>,
) {
    ui.heading("Audio");
    ui.add_space(8.0);
    egui::Grid::new("audio-output-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Sample rate");
            egui::ComboBox::from_id_salt("sample-rate")
                .selected_text(format!("{:.0} Hz", settings.sample_rate))
                .show_ui(ui, |ui| {
                    for rate in [44_100.0, 48_000.0, 96_000.0] {
                        ui.selectable_value(
                            &mut settings.sample_rate,
                            rate,
                            format!("{rate:.0} Hz"),
                        );
                    }
                });
            ui.end_row();

            ui.label("Buffer latency");
            egui::ComboBox::from_id_salt("audio-latency")
                .selected_text(format!("{} ms", settings.audio_latency_ms))
                .show_ui(ui, |ui| {
                    for ms in [15, 30, 60, 100, 150, 250] {
                        ui.selectable_value(&mut settings.audio_latency_ms, ms, format!("{ms} ms"));
                    }
                });
            ui.end_row();

            ui.label("Resampler");
            egui::ComboBox::from_id_salt("audio-quality")
                .selected_text(settings.audio_quality.label())
                .show_ui(ui, |ui| {
                    for quality in [
                        AudioQuality::Fast,
                        AudioQuality::Standard,
                        AudioQuality::High,
                    ] {
                        ui.selectable_value(&mut settings.audio_quality, quality, quality.label());
                    }
                });
            ui.end_row();

            ui.label("Output filter");
            let custom_model = settings.custom_audio_filter();
            egui::ComboBox::from_id_salt("audio-filter")
                .selected_text(settings.audio_filter.label())
                .show_ui(ui, |ui| {
                    for model in [
                        AudioFilterModel::NesRf,
                        AudioFilterModel::Crisp,
                        AudioFilterModel::Warm,
                        AudioFilterModel::Linear,
                    ] {
                        ui.selectable_value(&mut settings.audio_filter, model, model.label());
                    }
                    ui.selectable_value(&mut settings.audio_filter, custom_model, "Custom");
                });
            ui.end_row();
        });
    if matches!(settings.audio_filter, AudioFilterModel::Custom { .. }) {
        let mut custom = settings.custom_audio_filter();
        let AudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        } = &mut custom
        else {
            unreachable!()
        };
        ui.add(egui::Slider::new(high_pass_hz, 0.0..=1_000.0).text("High-pass (Hz, 0 = off)"));
        ui.add(
            egui::Slider::new(low_pass_hz, 2_000.0..=20_000.0)
                .text("Low-pass (Hz)")
                .logarithmic(true),
        );
        if let AudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        } = custom
        {
            settings.audio_high_pass = high_pass_hz;
            settings.audio_low_pass = low_pass_hz;
            settings.audio_filter = custom;
        }
    }
    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("Volume"));
    ui.checkbox(&mut settings.muted, "Mute");
    ui.add_space(10.0);
    ui.separator();
    ui.heading("Mixer");
    ui.horizontal(|ui| {
        ui.label("Audio source");
        egui::ComboBox::from_id_salt("audio-source")
            .selected_text(settings.audio_source.label())
            .show_ui(ui, |ui| {
                for source in [
                    AudioSource::Both,
                    AudioSource::Internal,
                    AudioSource::Expansion,
                    AudioSource::None,
                ] {
                    ui.selectable_value(&mut settings.audio_source, source, source.label());
                }
            });
    });
    ui.label(
        RichText::new(
            "Per-channel gains. Muting keeps the slider value so it can be restored;              the expansion row covers VRC6/7, MMC5, Namco 163, 5B and FDS audio.",
        )
        .weak()
        .small(),
    );
    const CHANNELS: [&str; 6] = [
        "Pulse 1",
        "Pulse 2",
        "Triangle",
        "Noise",
        "DMC",
        "Expansion",
    ];
    for (index, name) in CHANNELS.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.channel_mutes[index], "M");
            ui.label(format!("{name:9}"));
            ui.add(
                egui::Slider::new(&mut settings.channel_gains[index], 0.0..=1.0).show_value(true),
            );
        });
    }
    if ui.button("Reset mixer").clicked() {
        settings.channel_gains = [1.0; 6];
        settings.channel_mutes = [false; 6];
    }
    ui.add_space(10.0);
    match audio {
        Some(audio) => {
            ui.label(
                RichText::new(format!(
                    "Output: {:.0} Hz, {} channel(s), {}, {} underruns, {} dropped samples",
                    audio.sample_rate(),
                    audio.channels(),
                    if audio.is_active() {
                        "stream running"
                    } else {
                        "stream paused"
                    },
                    audio.underruns(),
                    audio.dropped()
                ))
                .weak()
                .small(),
            );
            ui.label(
                RichText::new(format!(
                    "Buffered: {:.1} ms (target {} ms, {:.0} samples queued)",
                    audio.queued_ms(),
                    audio.latency_ms(),
                    audio.queued_samples() as f32
                ))
                .weak()
                .small(),
            );
            ui.label(
                RichText::new(
                    "If the device does not support the selected rate, the device rate is used and the core resamples.",
                )
                .weak()
                .small(),
            );
        }
        None => {
            ui.colored_label(
                Color32::from_rgb(0xE0, 0x80, 0x60),
                format!(
                    "No audio output: {}",
                    audio_error.unwrap_or("device unavailable")
                ),
            );
        }
    }
}

fn controllers_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    gamepads: Option<&mut Gamepads>,
    gamepad_error: Option<&str>,
    hotkey_capture: &mut Option<HotkeyAction>,
) {
    ui.heading("Controllers");
    ui.add_space(8.0);
    let empty_names = [None, None, None, None];
    let pad_names = gamepads
        .as_ref()
        .map_or(empty_names, |gamepads| gamepads.names());
    if pad_names.iter().flatten().next().is_some() {
        let detected: Vec<String> = pad_names
            .iter()
            .enumerate()
            .filter_map(|(slot, name)| name.as_ref().map(|name| format!("{}: {name}", slot + 1)))
            .collect();
        ui.label(
            RichText::new(format!("Detected gamepads: {}", detected.join("  ·  ")))
                .weak()
                .small(),
        );
    } else if gamepad_error.is_none() {
        ui.label(
            RichText::new("No SDL3 gamepads detected yet.")
                .weak()
                .small(),
        );
    }
    if let Some(err) = gamepad_error {
        ui.colored_label(
            Color32::from_rgb(0xE0, 0x80, 0x60),
            format!("SDL3 gamepad unavailable: {err}"),
        );
        ui.label(RichText::new("Keyboard input is still available.").weak());
        ui.add_space(8.0);
    }
    ui.horizontal(|ui| {
        if ui.button("Restore default controls").clicked() {
            settings.pad_mapping.reset();
            settings.hotkeys = crate::settings::Hotkeys::default();
            settings.port1_input = PortInput::Auto;
            settings.port2_input = PortInput::Auto;
            settings.port1_device = PortDevice::Gamepad;
            settings.port2_device = PortDevice::Gamepad;
            settings.port_disabled = [false; 2];
            settings.pad_disabled = [false; 4];
            settings.pad_ports = [0, 1, 2, 3];
            settings.pad_deadzone = 12_000;
            settings.turbo_enabled = false;
        }
        ui.label(
            RichText::new("Defaults: keyboard on port 1, gamepad matrix, standard NES mapping.")
                .weak()
                .small(),
        );
    });
    ui.add_space(4.0);
    let old_port1 = settings.port1_input;
    let old_port2 = settings.port2_input;
    ui.horizontal(|ui| {
        ui.label("Port 1 connected");
        port_input_combo(ui, "port1-input", &mut settings.port1_input, &pad_names);
        ui.label("Protocol");
        device_combo(ui, "port1-device", &mut settings.port1_device);
    });
    ui.horizontal(|ui| {
        ui.label("Port 2 connected");
        port_input_combo(ui, "port2-input", &mut settings.port2_input, &pad_names);
        ui.label("Protocol");
        device_combo(ui, "port2-device", &mut settings.port2_device);
    });
    if settings.port1_input != old_port1 {
        apply_port_input(settings, 0);
    }
    if settings.port2_input != old_port2 {
        apply_port_input(settings, 1);
    }
    ui.label(
        RichText::new(
            "Connected chooses the host device; Protocol chooses the NES hardware. \
             Mouse options set the Zapper/Paddle protocol automatically, and None disables the port.",
        )
        .weak()
        .small(),
    );
    match (settings.port1_device, settings.port2_device) {
        (PortDevice::FourScore, _) | (_, PortDevice::FourScore) => ui.label(
            RichText::new(
                "Four Score: players 1-4 are read from gamepads 1-4 (it occupies both ports).",
            )
            .weak()
            .small(),
        ),
        (PortDevice::Paddle, _) | (_, PortDevice::Paddle) => ui.label(
            RichText::new("Paddle: the mouse steers horizontally, the left button fires.")
                .weak()
                .small(),
        ),
        (PortDevice::Mouse, _) | (_, PortDevice::Mouse) => ui.label(
            RichText::new(
                "Mouse: move and click inside the viewport for the SNES/Subor 32-bit report.",
            )
            .weak()
            .small(),
        ),
        (PortDevice::PowerPad | PortDevice::FamilyTrainer, _)
        | (_, PortDevice::PowerPad | PortDevice::FamilyTrainer) => ui.label(
            RichText::new("Power Pad: click the 12-pad overlay in the viewport.")
                .weak()
                .small(),
        ),
        (PortDevice::FamilyKeyboard, _) | (_, PortDevice::FamilyKeyboard) => ui.label(
            RichText::new("Family BASIC keyboard: host keys map to the documented matrix.")
                .weak()
                .small(),
        ),
        _ => ui.label(""),
    };
    ui.add_space(8.0);
    ui.checkbox(&mut settings.gamepad_enabled, "Enable SDL3 gamepads");
    if let Some(gamepads) = gamepads {
        if let Some((button, pad)) = gamepads.take_captured() {
            settings.pad_mapping.set(button, pad);
        }
        let names = gamepads.names();
        let connected = gamepads.connected();
        ui.add_space(6.0);
        if connected == 0 {
            ui.label(
                RichText::new("No gamepads detected. Connect one and it will appear here.").weak(),
            );
        } else {
            egui::Grid::new("gamepad-list")
                .num_columns(2)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    for (index, name) in names.iter().enumerate() {
                        if let Some(name) = name {
                            ui.label(format!("Player {}", index + 1));
                            ui.label(RichText::new(name).strong());
                            ui.end_row();
                        }
                    }
                });
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "NES port 1 — {}",
                port_input_description(settings.port1_input, &pad_names)
            ))
            .strong(),
        );
        ui.add_space(4.0);
        egui::Grid::new("controller-map")
            .num_columns(3)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                ui.label("NES");
                ui.label("Keyboard");
                ui.label("Gamepad");
                ui.end_row();
                for (button, key) in [
                    ("D-pad", "Arrow keys"),
                    ("A", "X"),
                    ("B", "Z"),
                    ("Start", "Enter"),
                    ("Select", "Shift"),
                ] {
                    ui.label(button);
                    ui.label(RichText::new(key).strong());
                    ui.label("");
                    ui.end_row();
                }
            });
        ui.add_space(8.0);
        ui.label("Gamepad bindings");
        egui::Grid::new("gamepad-bindings")
            .num_columns(3)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for button in NesButton::ALL {
                    ui.label(button.label());
                    ui.label(RichText::new(button_name(settings.pad_mapping.get(button))).strong());
                    let label = if gamepads.capturing() {
                        "Press a button..."
                    } else {
                        "Bind"
                    };
                    if ui
                        .add_enabled(!gamepads.capturing(), egui::Button::new(label))
                        .clicked()
                    {
                        gamepads.begin_capture(button);
                    }
                    ui.end_row();
                }
            });
        ui.horizontal(|ui| {
            if ui.button("Reset gamepad bindings").clicked() {
                settings.pad_mapping.reset();
            }
            ui.label(
                RichText::new("Bindings apply immediately; analog sticks also act as the D-pad.")
                    .weak()
                    .small(),
            );
        });
        ui.add(egui::Slider::new(&mut settings.pad_deadzone, 0..=32_000).text("Stick dead zone"));
        ui.add_space(8.0);
        ui.checkbox(&mut settings.turbo_enabled, "Auto-fire A/B on gamepad");
        if settings.turbo_enabled {
            ui.add(egui::Slider::new(&mut settings.turbo_hz, 2.0..=30.0).text("Auto-fire Hz"));
        }
    }
    ui.add_space(8.0);
    ui.label("Keyboard shortcuts");
    egui::Grid::new("hotkeys")
        .num_columns(3)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for action in HotkeyAction::ALL {
                ui.label(action.label());
                ui.label(RichText::new(settings.hotkeys.get(action).name()).strong());
                let label = if *hotkey_capture == Some(action) {
                    "Press a key..."
                } else {
                    "Set"
                };
                if ui.button(label).clicked() {
                    *hotkey_capture = Some(action);
                }
                ui.end_row();
            }
        });
    if ui.button("Reset shortcuts").clicked() {
        settings.hotkeys = crate::settings::Hotkeys::default();
    }
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.per_game_input,
        "Keep gamepad bindings per game",
    );
    ui.checkbox(
        &mut settings.show_input_display,
        "Show input display overlay",
    );
    ui.checkbox(
        &mut settings.show_touch_controls,
        "Show on-screen touch controller",
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Device changes apply immediately. Zapper, paddle and mouse follow the pointer inside the viewport.",
        )
        .weak()
        .small(),
    );
    ui.label(
        RichText::new(
            "Controller navigation: D-pad selects in the library, A plays, B clears, \
             Select toggles grid/list, and Start+Select opens the menu (it also pauses \
             a running game).",
        )
        .weak()
        .small(),
    );
}

/// Human label for a controller-port device.
fn port_input_description(input: PortInput, names: &[Option<String>; 4]) -> String {
    match input {
        PortInput::Auto => "Auto (keyboard + gamepads)".into(),
        PortInput::None => "None".into(),
        PortInput::Keyboard => "Keyboard".into(),
        PortInput::Gamepad(slot) => match names.get(slot).and_then(|name| name.as_ref()) {
            Some(name) => format!("Gamepad {} — {name}", slot + 1),
            None => format!("Gamepad {}", slot + 1),
        },
        PortInput::MouseZapper => "Mouse (Zapper)".into(),
        PortInput::MousePaddle => "Mouse (Paddle)".into(),
        PortInput::Mouse => "Mouse (SNES/Subor)".into(),
    }
}

fn port_input_combo(
    ui: &mut egui::Ui,
    id: &str,
    input: &mut PortInput,
    names: &[Option<String>; 4],
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(port_input_description(*input, names))
        .show_ui(ui, |ui| {
            for option in [
                PortInput::Auto,
                PortInput::None,
                PortInput::Keyboard,
                PortInput::Gamepad(0),
                PortInput::Gamepad(1),
                PortInput::Gamepad(2),
                PortInput::Gamepad(3),
                PortInput::MouseZapper,
                PortInput::MousePaddle,
                PortInput::Mouse,
            ] {
                ui.selectable_value(input, option, port_input_description(option, names));
            }
        });
}

fn apply_port_input(settings: &mut Settings, port: usize) {
    let input = if port == 0 {
        settings.port1_input
    } else {
        settings.port2_input
    };
    let device = if port == 0 {
        &mut settings.port1_device
    } else {
        &mut settings.port2_device
    };
    match input {
        PortInput::None => settings.port_disabled[port] = true,
        PortInput::MouseZapper => {
            settings.port_disabled[port] = false;
            *device = PortDevice::Zapper;
        }
        PortInput::MousePaddle => {
            settings.port_disabled[port] = false;
            *device = PortDevice::Paddle;
        }
        PortInput::Mouse => {
            settings.port_disabled[port] = false;
            *device = PortDevice::Mouse;
        }
        PortInput::Auto | PortInput::Keyboard | PortInput::Gamepad(_) => {
            settings.port_disabled[port] = false;
            if matches!(
                *device,
                PortDevice::Zapper | PortDevice::Paddle | PortDevice::Mouse
            ) {
                *device = PortDevice::Gamepad;
            }
        }
    }
}

fn device_label(device: PortDevice) -> &'static str {
    match device {
        PortDevice::Gamepad => "Gamepad",
        PortDevice::Zapper => "Zapper",
        PortDevice::Paddle => "Paddle",
        PortDevice::Mouse => "Mouse (SNES/Subor)",
        PortDevice::PowerPad => "Power Pad",
        PortDevice::FamilyTrainer => "Family Trainer",
        PortDevice::FamilyKeyboard => "Family BASIC keyboard",
        PortDevice::SuborKeyboard => "Subor keyboard",
        PortDevice::FourScore => "Four Score (4 players)",
        PortDevice::Microphone => "Famicom microphone",
        PortDevice::Tape => "Family BASIC data recorder",
        PortDevice::MiraclePiano => "Miracle Piano (MIDI)",
    }
}

/// Device picker for one controller port.
fn device_combo(ui: &mut egui::Ui, id: &str, device: &mut PortDevice) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(device_label(*device))
        .show_ui(ui, |ui| {
            for option in [
                PortDevice::Gamepad,
                PortDevice::Zapper,
                PortDevice::Paddle,
                PortDevice::Mouse,
                PortDevice::PowerPad,
                PortDevice::FamilyTrainer,
                PortDevice::FamilyKeyboard,
                PortDevice::SuborKeyboard,
                PortDevice::FourScore,
                PortDevice::Microphone,
                PortDevice::Tape,
                PortDevice::MiraclePiano,
            ] {
                if ui
                    .selectable_label(*device == option, device_label(option))
                    .clicked()
                {
                    *device = option;
                }
            }
        });
}

fn paths_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Paths");
    ui.add_space(8.0);
    let root = paths::data_root();
    ui.label("Data folder");
    ui.horizontal(|ui| {
        ui.monospace(root.display().to_string());
        if ui.button("Copy path").clicked() {
            ui.ctx().copy_text(root.display().to_string());
        }
        if ui.button("Use portable defaults").clicked() {
            settings.rom_dir = paths::roms_dir();
            settings.save_dir = paths::saves_dir();
            settings.fds_bios = paths::fds_bios_path();
        }
    });
    ui.label(
        RichText::new(if paths::is_portable() {
            "Portable: move this whole folder to another machine. Relative paths below are stored relative to it."
        } else {
            "Move the folder with unko.conf, roms/ and saves/ to another machine; paths are stored relative to it."
        })
        .weak()
        .small(),
    );
    ui.label(
        RichText::new(
            "Override the data folder with --data-dir DIR or UNKO_HOME. Absolute paths below are kept absolute.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.separator();
    ui.label("ROM folder");
    ui.horizontal(|ui| {
        let mut rom_dir = settings.rom_dir.display().to_string();
        if ui.text_edit_singleline(&mut rom_dir).changed() {
            settings.rom_dir = paths::resolve(Path::new(&rom_dir));
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&settings.rom_dir)
                .pick_folder()
            {
                settings.rom_dir = path;
            }
        }
    });
    ui.label("Save/state folder");
    ui.horizontal(|ui| {
        let mut save_dir = settings.save_dir.display().to_string();
        if ui.text_edit_singleline(&mut save_dir).changed() {
            settings.save_dir = paths::resolve(Path::new(&save_dir));
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&settings.save_dir)
                .pick_folder()
            {
                settings.save_dir = path;
            }
        }
    });
    ui.label("FDS BIOS (disksys.rom, 8 KiB; never bundled)");
    ui.horizontal(|ui| {
        let mut bios = settings.fds_bios.display().to_string();
        if ui.text_edit_singleline(&mut bios).changed() {
            settings.fds_bios = paths::resolve(Path::new(&bios));
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(
                    settings
                        .fds_bios
                        .parent()
                        .filter(|path| !path.as_os_str().is_empty())
                        .unwrap_or(&settings.save_dir),
                )
                .add_filter("FDS BIOS", &["rom", "bin"])
                .pick_file()
            {
                settings.fds_bios = path;
            }
        }
    });
    ui.add_space(12.0);
    ui.separator();
    ui.heading("Library roots");
    ui.label(
        RichText::new(
            "Additional folders tagged by system. Only NES roots are scanned in this milestone; \
             the SNES/N64 tags are placeholders for the multi-system library.",
        )
        .weak()
        .small(),
    );
    let mut remove = None;
    for (index, root) in settings.library_roots.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut system = root.system.clone();
            egui::ComboBox::from_id_salt(("root-system", index))
                .selected_text(system_label(&system))
                .show_ui(ui, |ui| {
                    for candidate in ["nes", "snes", "n64"] {
                        ui.selectable_value(
                            &mut system,
                            candidate.to_string(),
                            system_label(candidate),
                        );
                    }
                });
            root.system = system;
            let mut path = root.path.display().to_string();
            if ui.text_edit_singleline(&mut path).changed() {
                root.path = PathBuf::from(path);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&root.path)
                    .pick_folder()
                {
                    root.path = path;
                }
            }
            if ui.button("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        settings.library_roots.remove(index);
    }
    if ui.button("Add library root").clicked() {
        settings.library_roots.push(crate::settings::LibraryRoot {
            system: "nes".into(),
            path: settings.rom_dir.clone(),
        });
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("Changing the ROM folder rescans the library when you close Settings.")
            .weak()
            .small(),
    );
}

fn nes_tab(ui: &mut egui::Ui, settings: &mut Settings, entry: Option<&GameEntry>) {
    ui.heading("NES");
    ui.add_space(8.0);
    egui::ComboBox::from_label("Region")
        .selected_text(player::region_label(settings.region))
        .show_ui(ui, |ui| {
            for region in [Region::Auto, Region::Ntsc, Region::Pal, Region::Dendy] {
                ui.selectable_value(&mut settings.region, region, player::region_label(region));
            }
        });
    ui.add_space(8.0);
    match entry.and_then(|entry| entry.header.map(|header| (entry, header))) {
        Some((entry, header)) => {
            ui.label(RichText::new(&entry.title).strong());
            egui::Grid::new("nes-header")
                .num_columns(2)
                .spacing([24.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Mapper");
                    ui.label(format!("{}:{}", header.mapper, header.submapper));
                    ui.end_row();
                    ui.label("PRG");
                    ui.label(format!("{} KiB", header.prg_kib));
                    ui.end_row();
                    ui.label("CHR");
                    ui.label(format!("{} KiB", header.chr_kib));
                    ui.end_row();
                    ui.label("Battery");
                    ui.label(if header.battery { "yes" } else { "no" });
                    ui.end_row();
                    ui.label("Format");
                    ui.label(if header.nes2 { "NES 2.0" } else { "iNES" });
                    ui.end_row();
                });
        }
        None => {
            ui.label("Select a game in the library to see its header details.");
        }
    }
    ui.add_space(12.0);
    ui.label(
        RichText::new(
            "Region changes apply to the next launch. Power-on RAM is deterministic (zeroed) for state/netplay equivalence.",
        )
        .weak()
        .small(),
    );
}

fn advanced_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Advanced");
    ui.add_space(8.0);
    ui.label("Core: Unko NES accuracy core");
    ui.label("State format: UNKOST20, ROM-fingerprinted and core-tagged");
    ui.label("Settings file: unko.conf");
    ui.add_space(10.0);
    ui.separator();
    ui.heading("Soft-reset accuracy");
    ui.horizontal(|ui| {
        ui.label("PPU reset behavior");
        egui::ComboBox::from_id_salt("ppu-reset")
            .selected_text(settings.ppu_reset.label())
            .show_ui(ui, |ui| {
                for behavior in [PpuResetBehavior::Famicom, PpuResetBehavior::Nes] {
                    ui.selectable_value(&mut settings.ppu_reset, behavior, behavior.label());
                }
            });
    });
    ui.checkbox(
        &mut settings.mmc3_alternate,
        "Alternate MMC3 IRQ revision (MMC3A/NEC)",
    );
    ui.label(
        RichText::new(
            "Use for boards that raise a single IRQ when reloading to zero (some Crystalis/MMC6 dumps). Normal MMC3B/C behavior is the default.",
        )
        .weak()
        .small(),
    );
    ui.horizontal(|ui| {
        ui.label("FDS disk speed");
        egui::ComboBox::from_id_salt("fds-speed")
            .selected_text(format!("{}x", settings.fds_speed))
            .show_ui(ui, |ui| {
                for speed in [1u8, 2, 4, 8] {
                    ui.selectable_value(&mut settings.fds_speed, speed, format!("{speed}x"));
                }
            });
    });
    ui.checkbox(
        &mut settings.fds_auto_side,
        "Automatically swap FDS sides at end of side",
    );
    ui.label(
        RichText::new(
            "Disk speed scales the FDS byte-transfer cadence. Auto-swap is a convenience for multi-side images; leave it off for games that manage sides explicitly.",
        )
        .weak()
        .small(),
    );
    ui.horizontal(|ui| {
        ui.label("RAM clear");
        egui::ComboBox::from_id_salt("ram-clear")
            .selected_text(settings.ram_clear.label())
            .show_ui(ui, |ui| {
                for mode in [
                    RamClear::Keep,
                    RamClear::Zero,
                    RamClear::Fill,
                    RamClear::Random,
                ] {
                    ui.selectable_value(&mut settings.ram_clear, mode, mode.label());
                }
            });
    });
    ui.label(
        RichText::new(
            "These apply on the next Emulation -> Reset (or the reset hotkey), matching the \
             MiSTer options. A power-on load starts from zeroed work RAM.",
        )
        .weak()
        .small(),
    );
    ui.add_space(10.0);
    ui.separator();
    ui.heading("ROM headers");
    ui.label(
        RichText::new(
            "Auto region needs a NES 2.0 header; iNES images default to NTSC, as MiSTer warns. \
             Bulk header repair is an external tool, not part of the core.",
        )
        .weak()
        .small(),
    );
    ui.label(
        RichText::new("Logging, run-ahead and rewind controls land with the next milestone.")
            .weak()
            .small(),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotAction {
    Save,
    Load,
    Delete,
    Prune,
    ClearAll,
}

fn slot_placeholder(ui: &mut egui::Ui, label: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 60.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0_f32, Color32::from_gray(60)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        Color32::GRAY,
    );
}

const LIST_COLUMN_BITS: [(&str, u8); 7] = [
    ("System", 0x01),
    ("File", 0x02),
    ("Size", 0x04),
    ("Region", 0x08),
    ("Mapper", 0x10),
    ("Modified", 0x20),
    ("Played", 0x40),
];

fn column_cell(entry: &crate::library::GameEntry, bit: u8, played_seconds: u64) -> String {
    match bit {
        0x01 => entry.system().to_string(),
        0x02 => entry.file_name.clone(),
        0x04 => entry.size_label(),
        0x08 => entry.region_label().to_string(),
        0x10 => entry.mapper_label(),
        0x40 => format_duration(played_seconds),
        _ => modified_label(entry.modified),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix_with(events: Vec<egui::Event>) -> [u8; 9] {
        let ctx = egui::Context::default();
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            modifiers,
            events,
            ..egui::RawInput::default()
        };
        let mut keys = [0u8; 9];
        let _ = ctx.run(input, |ctx| {
            keys = family_keyboard_matrix(ctx);
        });
        keys
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn family_keyboard_maps_letters_to_the_documented_matrix() {
        // A = row 6, column half 0, first key -> bit 3.
        let keys = matrix_with(vec![key_event(egui::Key::A, egui::Modifiers::NONE)]);
        assert_eq!(keys[6], 1 << 3);
        // 1 = row 7, half 1, second key -> bit 4 + (3 - 1) = 6.
        let keys = matrix_with(vec![key_event(egui::Key::Num1, egui::Modifiers::NONE)]);
        assert_eq!(keys[7], 1 << 6);
        // Enter = row 0, half 0, third key -> bit 1.
        let keys = matrix_with(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(keys[0], 1 << 1);
    }

    #[test]
    fn family_keyboard_does_not_press_both_shift_variants() {
        let keys = matrix_with(vec![key_event(egui::Key::Num2, egui::Modifiers::SHIFT)]);
        assert_eq!(keys[1] & (1 << 1), 1 << 1, "@ is pressed");
        assert_eq!(keys[7] & (1 << 7), 0, "2 is not pressed while shifted");
        let keys = matrix_with(vec![key_event(egui::Key::Minus, egui::Modifiers::SHIFT)]);
        assert_eq!(keys[1] & (1 << 4), 1 << 4, "_ is pressed");
        assert_eq!(keys[1] & (1 << 6), 0, "- is not pressed while shifted");
    }

    #[test]
    fn family_keyboard_maps_shifted_punctuation() {
        let keys = matrix_with(vec![key_event(egui::Key::Num2, egui::Modifiers::SHIFT)]);
        // "@" is row 1, half 0, third key -> bit 1, plus the 2 key and shift.
        assert_eq!(keys[1] & (1 << 1), 1 << 1);
        assert_eq!(keys[7] & (1 << 4), 1 << 4, "LSHIFT is held");
    }
}

#[cfg(test)]
mod controller_chord_tests {
    use super::controller_chord;
    use unko_core::InputState;

    #[test]
    fn only_start_and_select_together_open_the_menu() {
        let none = InputState::empty();
        assert!(!controller_chord(
            InputState::from_bits(InputState::START),
            none
        ));
        assert!(!controller_chord(
            InputState::from_bits(InputState::SELECT),
            none
        ));
        assert!(!controller_chord(
            InputState::from_bits(InputState::A),
            none
        ));
        let both = InputState::from_bits(InputState::START | InputState::SELECT);
        assert!(controller_chord(both, none));
        assert!(!controller_chord(
            both,
            InputState::from_bits(InputState::START | InputState::SELECT)
        ));
        assert!(!controller_chord(
            InputState::from_bits(InputState::START | InputState::SELECT | InputState::A),
            InputState::from_bits(InputState::START | InputState::SELECT)
        ));
    }
}
