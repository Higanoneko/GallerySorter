//! Ratatui 终端 UI 模块
//!
//! 提供基于 ratatui 的终端用户界面。

pub mod app;
pub mod components;
pub mod event;
pub mod labels;
pub mod screens;
pub mod state;
pub mod theme;
pub mod ui;

pub use app::TuiApp;
pub use event::{EventPoll, TuiEvent};
pub use state::{
    AppState, ConfigFormState, ConfigStep, ConfigWizardState, Effect, FormField, InputState,
    MenuItem, MenuState, ProgressState, Screen, SelectionState, SummaryState, TuiResult,
    WizardFlow, reset_to_main_menu, transition,
};
pub use theme::{Theme, theme};
pub use ui::{render, run_processing};
