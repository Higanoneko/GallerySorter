//! TUI 应用入口
//!
//! 提供 TUI 应用的创建与运行逻辑。

use crate::config::Config;
use crate::tui::event::{EventPoll, TuiEvent};
use crate::tui::state::{
    AppState, ConfigFormState, ConfigStep, ConfigWizardState, Effect, MenuItem, Screen, Selectable,
    TuiResult, WizardFlow, reset_to_main_menu, transition,
};
use crate::tui::ui::{render, run_processing};
use ratatui::DefaultTerminal;
use std::collections::VecDeque;
use std::path::PathBuf;

/// TUI 应用
#[derive(Debug)]
pub struct TuiApp {
    /// 终端
    pub terminal: DefaultTerminal,
    /// 事件轮询器
    pub event_poll: EventPoll,
    /// 应用状态
    pub state: AppState,
    /// 日志路径
    log_path: Option<PathBuf>,
    /// 待执行效果队列
    effect_queue: VecDeque<Effect>,
}

impl TuiApp {
    /// 创建 TUI 应用
    pub fn new() -> std::io::Result<Self> {
        let terminal = ratatui::init();
        let event_poll = EventPoll::default();
        let state = AppState::default();

        Ok(Self {
            terminal,
            event_poll,
            state,
            log_path: None,
            effect_queue: VecDeque::new(),
        })
    }

    /// 设置日志路径
    pub fn set_log_path(&mut self, path: PathBuf) {
        self.log_path = Some(path);
    }

    /// 运行应用
    pub fn run(&mut self) -> std::io::Result<Option<TuiResult>> {
        render(&mut self.terminal, &mut self.state)?;

        loop {
            // 执行效果队列
            if !self.effect_queue.is_empty() {
                self.execute_effects()?;
                render(&mut self.terminal, &mut self.state)?;
                continue;
            }

            // 收集事件
            match self.event_poll.next() {
                TuiEvent::Resize(_, _) => {
                    render(&mut self.terminal, &mut self.state)?;
                }
                TuiEvent::CtrlC => {
                    self.state.should_exit = true;
                    self.state.current_screen = Screen::Exit;
                    render(&mut self.terminal, &mut self.state)?;
                    break;
                }
                event => {
                    // 转换
                    if self.handle_event(event)? {
                        if self.state.current_screen == Screen::Summary {
                            reset_to_main_menu(&mut self.state);
                            render(&mut self.terminal, &mut self.state)?;
                            continue;
                        }
                        break;
                    }
                    // 重绘
                    render(&mut self.terminal, &mut self.state)?;
                }
            }
        }

        ratatui::restore();
        Ok(self.state.result.take())
    }

    fn handle_event(&mut self, event: TuiEvent) -> std::io::Result<bool> {
        match self.state.current_screen {
            Screen::MainMenu => self.handle_main_menu(event),
            Screen::ConfigWizard => self.handle_config_wizard(event),
            Screen::Progress => self.handle_progress(event),
            Screen::Summary => self.handle_summary(event),
            Screen::Exit => self.handle_exit(event),
        }
    }

    fn handle_main_menu(&mut self, event: TuiEvent) -> std::io::Result<bool> {
        match event {
            TuiEvent::Up | TuiEvent::Left => self.state.menu_state.prev(),
            TuiEvent::Down | TuiEvent::Right => self.state.menu_state.next(),
            TuiEvent::Enter => {
                let item = MenuItem::iter().nth(self.state.menu_state.selected());
                match item {
                    Some(MenuItem::Exit) => return Ok(true),
                    Some(MenuItem::RunDirect) => {
                        self.state.current_screen = Screen::ConfigWizard;
                        self.state.config_wizard = ConfigWizardState::new();
                        self.state.config_wizard.step = ConfigStep::ConfigForm;
                        self.state.config_wizard.flow = WizardFlow::RunDirect;
                        self.state.config_wizard.form_state.selected_field = 0;
                    }
                    Some(MenuItem::RunConfig) => {
                        self.state.current_screen = Screen::ConfigWizard;
                        self.state.config_wizard = ConfigWizardState::new();
                        self.state.config_wizard.step = ConfigStep::ConfigSelect;
                        self.state.config_wizard.flow = WizardFlow::RunFromConfig {
                            need_modify_confirm: true,
                        };
                        self.effect_queue.push_back(Effect::RefreshConfigs);
                    }
                    Some(MenuItem::CreateConfig) => {
                        self.state.current_screen = Screen::ConfigWizard;
                        self.state.config_wizard = ConfigWizardState::new();
                        self.state.config_wizard.step = ConfigStep::ConfigForm;
                        self.state.config_wizard.flow = WizardFlow::CreateConfig;
                        self.state.config_wizard.form_state.selected_field = 0;
                    }
                    None => {}
                }
            }
            TuiEvent::Escape => return Ok(true),
            _ => {}
        }
        Ok(false)
    }

    fn handle_config_wizard(&mut self, event: TuiEvent) -> std::io::Result<bool> {
        let effects = transition(&mut self.state, event);
        self.effect_queue.extend(effects);
        Ok(false)
    }

    /// 在边缘执行效果队列（唯一执行副作用的地方）
    fn execute_effects(&mut self) -> std::io::Result<()> {
        while let Some(effect) = self.effect_queue.pop_front() {
            match effect {
                Effect::LoadConfig(path) => match Config::load_from_file(&path) {
                    Ok(config) => {
                        self.state.config_wizard.init_from_config(&config, &path);
                        self.state.config_wizard.config_path = Some(path);
                        self.state.config_wizard.error_message = None;
                        self.state.config_wizard.form_state = ConfigFormState::new();
                        self.state.config_wizard.flow = WizardFlow::RunFromConfig {
                            need_modify_confirm: true,
                        };
                        self.state.config_wizard.step = ConfigStep::Summary;
                    }
                    Err(err) => {
                        self.state.config_wizard.error_message = Some(err.to_string());
                    }
                },
                Effect::SaveConfig(name) => {
                    let _ = self.save_config_to_disk(&name);
                }
                Effect::RefreshConfigs => {
                    self.refresh_configs_from_disk();
                }
                Effect::RunProcessing {
                    config,
                    config_name,
                } => {
                    let run_config = (*config).clone();
                    self.state.current_screen = Screen::Progress;
                    render(&mut self.terminal, &mut self.state)?;

                    let summary_state =
                        run_processing(&mut self.terminal, run_config, self.log_path.clone())?;

                    self.state.summary_state = summary_state;
                    self.state.current_screen = Screen::Summary;
                    self.state.result = Some(TuiResult {
                        config: *config,
                        config_name,
                    });
                }
            }
        }
        Ok(())
    }

    /// 保存配置到磁盘（含 current_exe() 路径解析，IO 边缘执行）
    fn save_config_to_disk(&mut self, name: &str) -> Result<PathBuf, String> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let config_dir = exe_dir.join("Config");

        std::fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;

        let config_path = config_dir.join(name).with_extension("toml");
        let config = self.state.config_wizard.build_config();
        let content = toml::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        std::fs::write(&config_path, content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        self.state.config_wizard.config_path = Some(config_path.clone());
        Ok(config_path)
    }

    /// 扫描 Config 目录（含 current_exe() 路径解析，IO 边缘执行）
    fn refresh_configs_from_disk(&mut self) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let config_dir = exe_dir.join("Config");

        let paths = if !config_dir.exists() {
            Vec::new()
        } else {
            std::fs::read_dir(&config_dir)
                .ok()
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.path()
                                .extension()
                                .map(|ext| ext == std::ffi::OsStr::new("toml"))
                                .unwrap_or(false)
                        })
                        .map(|e| e.path())
                        .collect()
                })
                .unwrap_or_default()
        };

        self.state.config_wizard.set_configs_from_paths(paths);
    }

    fn handle_progress(&mut self, _event: TuiEvent) -> std::io::Result<bool> {
        Ok(false)
    }

    fn handle_summary(&mut self, event: TuiEvent) -> std::io::Result<bool> {
        match event {
            TuiEvent::Enter | TuiEvent::Escape => {
                reset_to_main_menu(&mut self.state);
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_exit(&mut self, event: TuiEvent) -> std::io::Result<bool> {
        match event {
            TuiEvent::Char('y') | TuiEvent::Char('Y') => return Ok(true),
            TuiEvent::Char('n') | TuiEvent::Char('N') | TuiEvent::Escape => {
                self.state.current_screen = Screen::MainMenu;
                self.state.should_exit = false;
            }
            _ => {}
        }
        Ok(false)
    }
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new().expect("Failed to initialize TUI")
    }
}
