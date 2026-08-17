//! 配置向导状态

use crate::config::{
    ClassificationRule, Config, EnumOption, FileOperation, MonthFormat, ProcessingMode,
};
use crate::tui::event::TuiEvent;
use crate::tui::labels::{
    bool_label, classification_label, file_operation_label, month_format_label,
    processing_mode_label,
};
use crate::tui::state::app::{AppState, reset_to_main_menu};
use crate::tui::state::input::InputState;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// 状态层副作用
///
/// 转换函数不执行任何 IO，所有副作用以 `Effect` 表达，
/// 由 `app.rs` 在边缘执行。
#[derive(Debug, Clone)]
pub enum Effect {
    /// 加载指定配置文件（当前行为：加载后初始化表单并进入摘要）
    LoadConfig(PathBuf),
    /// 保存配置（携带配置名称与配置快照，避免状态被重置后丢失内容）
    SaveConfig { name: String, config: Box<Config> },
    /// 刷新配置列表
    RefreshConfigs,
    /// 启动处理（携带配置与配置名称）
    RunProcessing {
        config: Box<Config>,
        config_name: Option<String>,
    },
}

/// 向导流程
///
/// 取代原先 4 个布尔标志（`skip_confirm_run` / `from_config_select` /
/// `need_modify_confirm` / `config_saved`），三条流程在类型层面互斥。
/// `RunFromConfig` 携带“是否需要修改确认”的运行时状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WizardFlow {
    /// 默认状态（等价于原全 false 标志组）
    #[default]
    CreateConfig,
    /// 直接输入参数运行流（填表单 → 摘要 → 不保存配置、直接执行处理）
    RunDirect,
    /// 使用已有配置运行流（选择配置 → 可修改 → 确认后执行处理）
    RunFromConfig { need_modify_confirm: bool },
}

/// 配置向导转换函数
///
/// 纯转换：只修改状态并返回副作用列表，不执行任何 IO。
/// 行为与 app.rs 原 `handle_config_wizard` 逐分支一致。
pub fn transition(state: &mut AppState, event: TuiEvent) -> Vec<Effect> {
    let mut effects = Vec::new();
    let step = state.config_wizard.step.clone();

    match event {
        TuiEvent::Up => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_to_start();
            } else {
                match step {
                    ConfigStep::ConfigForm => state.config_wizard.navigate_form_prev(),
                    ConfigStep::ConfigSelect | ConfigStep::ConfirmRun => {
                        state.config_wizard.navigate_prev()
                    }
                    _ => {}
                }
            }
        }
        TuiEvent::Down => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_to_end();
            } else {
                match step {
                    ConfigStep::ConfigForm => state.config_wizard.navigate_form_next(),
                    ConfigStep::ConfigSelect | ConfigStep::ConfirmRun => {
                        state.config_wizard.navigate_next()
                    }
                    _ => {}
                }
            }
        }
        TuiEvent::Left => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_left();
            } else {
                match step {
                    ConfigStep::ConfigForm => {
                        if !state.config_wizard.is_next_selected() {
                            state.config_wizard.toggle_current_field_prev();
                        }
                    }
                    ConfigStep::ConfigSelect | ConfigStep::ConfirmRun => {
                        state.config_wizard.navigate_prev();
                    }
                    _ => {}
                }
            }
        }
        TuiEvent::Right => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_right();
            } else {
                match step {
                    ConfigStep::ConfigForm => {
                        if !state.config_wizard.is_next_selected() {
                            state.config_wizard.toggle_current_field_next();
                        }
                    }
                    ConfigStep::ConfigSelect | ConfigStep::ConfirmRun => {
                        state.config_wizard.navigate_next();
                    }
                    _ => {}
                }
            }
        }
        TuiEvent::Enter => match step {
            ConfigStep::ConfigSelect => {
                if !state.config_wizard.can_confirm_config_select() {
                    return effects;
                }

                state.config_wizard.ensure_selection();
                let selected = state.config_wizard.selected_value();
                let selected_path = state.config_wizard.available_configs.get(selected).cloned();

                if let Some(path) = selected_path {
                    effects.push(Effect::LoadConfig(path));
                }
            }
            ConfigStep::ConfirmRun => {
                state.config_wizard.ensure_selection();
                let selected = state.config_wizard.selected_value();

                if state.config_wizard.is_select_config_flow() {
                    if selected == 0 {
                        state.config_wizard.flow = WizardFlow::RunFromConfig {
                            need_modify_confirm: false,
                        };
                        state.config_wizard.form_state = ConfigFormState::new();
                        state.config_wizard.step = ConfigStep::ConfigForm;
                    } else {
                        let config = state.config_wizard.build_config();
                        effects.push(Effect::RunProcessing {
                            config: Box::new(config),
                            config_name: Some(state.config_wizard.config_name.clone()),
                        });
                    }
                } else {
                    // CreateConfig 在确认步骤始终保存配置：
                    // 先快照配置再入队，避免后续 reset 清空表单后保存成默认值
                    let config = state.config_wizard.build_config();
                    effects.push(Effect::SaveConfig {
                        name: state.config_wizard.config_name.clone(),
                        config: Box::new(config.clone()),
                    });

                    if selected == 0 {
                        effects.push(Effect::RunProcessing {
                            config: Box::new(config),
                            config_name: Some(state.config_wizard.config_name.clone()),
                        });
                    } else {
                        reset_to_main_menu(state);
                    }
                }
            }
            ConfigStep::ConfigForm => {
                if state.config_wizard.is_in_input_mode() {
                    state.config_wizard.exit_input_mode_apply();
                } else if state.config_wizard.is_next_selected() {
                    if state.config_wizard.validate_form().is_ok() {
                        state.config_wizard.step = ConfigStep::Summary;
                    } else {
                        state.config_wizard.error_message =
                            state.config_wizard.validate_form().err();
                    }
                } else if let Some(field) = state.config_wizard.selected_form_field()
                    && field.is_input_field()
                {
                    state.config_wizard.enter_input_mode_for_field();
                }
            }
            ConfigStep::Summary => {
                if matches!(state.config_wizard.flow, WizardFlow::RunDirect)
                    || (state.config_wizard.is_select_config_flow()
                        && !state.config_wizard.needs_modify_confirm())
                {
                    let config = state.config_wizard.build_config();
                    let config_name = if matches!(state.config_wizard.flow, WizardFlow::RunDirect) {
                        None
                    } else {
                        Some(state.config_wizard.config_name.clone())
                    };
                    effects.push(Effect::RunProcessing {
                        config: Box::new(config),
                        config_name,
                    });
                } else {
                    state.config_wizard.step = ConfigStep::ConfirmRun;
                    if state.config_wizard.is_select_config_flow() {
                        state.config_wizard.set_selected(1);
                    }
                    state.config_wizard.ensure_selection();
                }
            }
        },
        TuiEvent::Char(c) => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_insert_char(c);
            }
        }
        TuiEvent::Backspace => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_backspace();
            }
        }
        TuiEvent::Delete => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_delete();
            }
        }
        TuiEvent::Home => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_to_start();
            }
        }
        TuiEvent::End => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.input_move_to_end();
            }
        }
        TuiEvent::Escape => {
            if state.config_wizard.is_in_input_mode() {
                state.config_wizard.exit_input_mode_cancel();
            } else {
                match step {
                    ConfigStep::ConfirmRun => {
                        state.config_wizard.step = ConfigStep::Summary;
                    }
                    ConfigStep::ConfigForm => {
                        if state.config_wizard.is_select_config_flow() {
                            state.config_wizard.step = ConfigStep::ConfigSelect;
                        } else {
                            reset_to_main_menu(state);
                        }
                    }
                    ConfigStep::Summary => {
                        if state.config_wizard.is_select_config_flow()
                            && state.config_wizard.needs_modify_confirm()
                        {
                            state.config_wizard.step = ConfigStep::ConfigSelect;
                        } else {
                            state.config_wizard.step = ConfigStep::ConfigForm;
                        }
                    }
                    ConfigStep::ConfigSelect => reset_to_main_menu(state),
                }
            }
        }
        TuiEvent::Tab if !state.config_wizard.is_in_input_mode() => match step {
            ConfigStep::ConfigForm => state.config_wizard.navigate_form_next(),
            ConfigStep::ConfigSelect | ConfigStep::ConfirmRun => {
                state.config_wizard.navigate_next()
            }
            _ => {}
        },
        _ => {}
    }

    effects
}

/// 枚举选择状态
#[derive(Debug, Clone, Copy)]
pub struct EnumSelection<E: EnumOption> {
    selected: E,
    _phantom: PhantomData<E>,
}

impl<E: EnumOption> EnumSelection<E> {
    /// 创建选择状态
    pub fn new() -> Self {
        Self {
            selected: E::default(),
            _phantom: PhantomData,
        }
    }

    /// 使用指定选项创建
    pub fn with_selected(selected: E) -> Self {
        Self {
            selected,
            _phantom: PhantomData,
        }
    }

    /// 获取当前选项
    pub fn selected(&self) -> E {
        self.selected
    }

    /// 获取当前索引
    pub fn index(&self) -> usize {
        self.selected.to_index()
    }

    /// 选择指定选项
    pub fn select(&mut self, value: E) {
        self.selected = value;
    }

    /// 根据索引选择
    pub fn select_by_index(&mut self, index: usize) {
        self.selected = E::from_index(index);
    }

    /// 可选项数量
    pub fn count(&self) -> usize {
        E::variants().len()
    }

    /// 选择下一个
    pub fn next(&mut self) {
        let count = self.count();
        let new_index = (self.selected.to_index() + 1) % count;
        self.selected = E::from_index(new_index);
    }

    /// 选择上一个
    pub fn prev(&mut self) {
        let count = self.count();
        let new_index = if self.selected.to_index() == 0 {
            count - 1
        } else {
            self.selected.to_index() - 1
        };
        self.selected = E::from_index(new_index);
    }
}

impl<E: EnumOption> Default for EnumSelection<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// 布尔值选择
#[derive(Debug, Clone, Copy, Default)]
pub struct BoolSelection(bool);

impl BoolSelection {
    /// 创建布尔选择
    pub fn new(value: bool) -> Self {
        Self(value)
    }

    /// 获取值
    pub fn value(&self) -> bool {
        self.0
    }

    /// 切换值
    pub fn toggle(&mut self) {
        self.0 = !self.0;
    }

    /// 可选项数量（固定 2）
    pub fn count(&self) -> usize {
        2
    }

    /// 当前索引
    pub fn index(&self) -> usize {
        if self.0 { 1 } else { 0 }
    }

    /// 根据索引设置
    pub fn select_by_index(&mut self, index: usize) {
        self.0 = index == 1;
    }

    /// 选择下一个
    pub fn next(&mut self) {
        self.toggle();
    }

    /// 选择上一个
    pub fn prev(&mut self) {
        self.toggle();
    }
}

/// 配置向导步骤
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConfigStep {
    /// 配置选择
    #[default]
    ConfigSelect,
    /// 配置表单
    ConfigForm,
    /// 配置摘要
    Summary,
    /// 确认运行
    ConfirmRun,
}

impl ConfigStep {
    /// 获取标题
    pub fn title(&self) -> String {
        match self {
            ConfigStep::ConfigSelect => rust_i18n::t!("available_configurations").to_string(),
            ConfigStep::ConfigForm => rust_i18n::t!("configuration_form").to_string(),
            ConfigStep::Summary => rust_i18n::t!("configuration_summary").to_string(),
            ConfigStep::ConfirmRun => rust_i18n::t!("proceed_instent").to_string(),
        }
    }

    /// 可选项数量
    pub fn option_count(&self) -> usize {
        match self {
            ConfigStep::ConfigSelect => 1,
            ConfigStep::ConfirmRun => 2,
            _ => 0,
        }
    }

    /// 获取选项列表
    pub fn options(&self) -> Vec<String> {
        match self {
            ConfigStep::ConfirmRun => vec![
                rust_i18n::t!("option_yes").to_string(),
                rust_i18n::t!("option_no").to_string(),
            ],
            _ => vec![],
        }
    }

    /// 获取下一步
    pub fn next(&self, _classification: ClassificationRule) -> Self {
        match self {
            ConfigStep::ConfigSelect => ConfigStep::ConfigForm,
            ConfigStep::ConfigForm => ConfigStep::Summary,
            ConfigStep::Summary => ConfigStep::ConfirmRun,
            ConfigStep::ConfirmRun => ConfigStep::ConfirmRun,
        }
    }
}

/// 表单字段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    /// 配置名称
    ConfigName,
    /// 输入目录
    InputDirs,
    /// 输出目录
    OutputDir,
    /// 排除目录
    ExcludeDirs,
    /// 处理模式
    ProcessingMode,
    /// 分类规则
    Classification,
    /// 月份格式
    MonthFormat,
    /// 按类型分类
    ClassifyByType,
    /// 文件操作
    FileOperation,
    /// 去重
    Deduplication,
    /// 试运行
    DryRun,
    /// 统一文件名
    UnifyFilenames,
    /// 保留标准相机名称
    PreserveStandardNames,
}

impl FormField {
    /// 字段数量
    pub fn count() -> usize {
        13
    }

    /// 获取全部字段
    pub fn all() -> &'static [FormField] {
        &[
            FormField::ConfigName,
            FormField::InputDirs,
            FormField::OutputDir,
            FormField::ExcludeDirs,
            FormField::ProcessingMode,
            FormField::Classification,
            FormField::MonthFormat,
            FormField::ClassifyByType,
            FormField::FileOperation,
            FormField::Deduplication,
            FormField::DryRun,
            FormField::UnifyFilenames,
            FormField::PreserveStandardNames,
        ]
    }

    /// 是否输入字段
    pub fn is_input_field(&self) -> bool {
        matches!(
            self,
            FormField::ConfigName
                | FormField::InputDirs
                | FormField::OutputDir
                | FormField::ExcludeDirs
        )
    }

    /// 是否选项字段
    pub fn is_option_field(&self) -> bool {
        !self.is_input_field()
    }

    /// 字段标签
    pub fn label(&self) -> String {
        match self {
            FormField::ConfigName => rust_i18n::t!("field_config_name").to_string(),
            FormField::InputDirs => rust_i18n::t!("field_input_dirs").to_string(),
            FormField::OutputDir => rust_i18n::t!("field_output_dir").to_string(),
            FormField::ExcludeDirs => rust_i18n::t!("field_exclude_dirs").to_string(),
            FormField::ProcessingMode => rust_i18n::t!("field_processing_mode").to_string(),
            FormField::Classification => rust_i18n::t!("field_classification").to_string(),
            FormField::MonthFormat => rust_i18n::t!("field_month_format").to_string(),
            FormField::ClassifyByType => rust_i18n::t!("field_classify_by_type").to_string(),
            FormField::FileOperation => rust_i18n::t!("field_file_operation").to_string(),
            FormField::Deduplication => rust_i18n::t!("field_deduplication").to_string(),
            FormField::DryRun => rust_i18n::t!("field_dry_run").to_string(),
            FormField::UnifyFilenames => rust_i18n::t!("field_unify_filenames").to_string(),
            FormField::PreserveStandardNames => {
                rust_i18n::t!("field_preserve_standard_names").to_string()
            }
        }
    }

    /// 获取显示值
    pub fn get_value_string(&self, state: &ConfigWizardState) -> String {
        match self {
            FormField::ConfigName => state.config_name.clone(),
            FormField::InputDirs => state.input_dirs.clone(),
            FormField::OutputDir => state.output_dir.clone(),
            FormField::ExcludeDirs => state.exclude_dirs.clone(),
            FormField::ProcessingMode => {
                processing_mode_label(state.processing_mode.selected()).to_string()
            }
            FormField::Classification => {
                classification_label(state.classification.selected()).to_string()
            }
            FormField::MonthFormat => month_format_label(state.month_format.selected()).to_string(),
            FormField::ClassifyByType => bool_label(state.classify_by_type.value()).to_string(),
            FormField::FileOperation => {
                file_operation_label(state.operation.selected()).to_string()
            }
            FormField::Deduplication => bool_label(state.deduplicate.value()).to_string(),
            FormField::DryRun => bool_label(state.dry_run.value()).to_string(),
            FormField::UnifyFilenames => bool_label(state.unify_filenames.value()).to_string(),
            FormField::PreserveStandardNames => {
                bool_label(state.preserve_standard_names.value()).to_string()
            }
        }
    }

    /// 是否可见
    pub fn is_visible(&self, state: &ConfigWizardState) -> bool {
        match self {
            FormField::ConfigName => !state.is_run_direct_flow(),
            FormField::MonthFormat => {
                state.classification.selected() == ClassificationRule::YearMonth
            }
            // 仅在启用“统一文件名”时展示，避免非 rename 流程出现无效选项
            FormField::PreserveStandardNames => state.unify_filenames.value(),
            _ => true,
        }
    }
}

/// 表单状态
#[derive(Debug, Default)]
pub struct ConfigFormState {
    /// 当前选中字段索引
    pub selected_field: usize,
    /// 是否处于输入模式
    pub in_input_mode: bool,
    /// 输入状态
    pub input: InputState,
    /// 表单滚动偏移
    pub scroll_offset: usize,
}

impl ConfigFormState {
    /// 创建表单状态
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取当前字段
    pub fn selected(&self) -> FormField {
        FormField::all()
            .get(self.selected_field)
            .copied()
            .unwrap_or(FormField::ConfigName)
    }

    /// 选择下一个字段
    pub fn next_field(&mut self, visible_count: usize) {
        self.selected_field = (self.selected_field + 1) % visible_count;
        self.auto_scroll();
    }

    /// 选择上一个字段
    pub fn prev_field(&mut self, visible_count: usize) {
        self.selected_field = if self.selected_field == 0 {
            visible_count.saturating_sub(1)
        } else {
            self.selected_field - 1
        };
        self.auto_scroll();
    }

    fn auto_scroll(&mut self) {
        use crate::tui::theme::config::FORM_VISIBLE_ROWS;
        if self.selected_field >= self.scroll_offset + FORM_VISIBLE_ROWS {
            self.scroll_offset = self.selected_field.saturating_sub(FORM_VISIBLE_ROWS - 1);
        } else if self.selected_field < self.scroll_offset {
            self.scroll_offset = self.selected_field;
        }
    }

    /// 进入输入模式
    pub fn enter_input_mode(&mut self, value: &str) {
        self.in_input_mode = true;
        self.input = InputState::with_value(value);
    }

    /// 退出输入模式
    pub fn exit_input_mode(&mut self) {
        self.in_input_mode = false;
        self.input.clear();
    }

    /// 获取输入值
    pub fn input_value(&self) -> &str {
        self.input.value()
    }

    /// 获取光标位置
    pub fn input_cursor(&self) -> usize {
        self.input.cursor_position()
    }

    /// 清空输入
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// 可见字段数量
    pub fn visible_fields_count(&self, state: &ConfigWizardState) -> usize {
        FormField::all()
            .iter()
            .filter(|f| f.is_visible(state))
            .count()
    }
}

/// 配置向导状态
#[derive(Debug, Default)]
pub struct ConfigWizardState {
    /// 当前步骤
    pub step: ConfigStep,
    /// 向导流程
    pub flow: WizardFlow,
    /// 输入目录
    pub input_dirs: String,
    /// 输出目录
    pub output_dir: String,
    /// 排除目录
    pub exclude_dirs: String,
    /// 处理模式
    pub processing_mode: EnumSelection<ProcessingMode>,
    /// 分类规则
    pub classification: EnumSelection<ClassificationRule>,
    /// 月份格式
    pub month_format: EnumSelection<MonthFormat>,
    /// 文件操作
    pub operation: EnumSelection<FileOperation>,
    /// 去重
    pub deduplicate: BoolSelection,
    /// 试运行
    pub dry_run: BoolSelection,
    /// 按类型分类
    pub classify_by_type: BoolSelection,
    /// 统一文件名
    pub unify_filenames: BoolSelection,
    /// 保留标准相机名称
    pub preserve_standard_names: BoolSelection,
    /// 配置名称
    pub config_name: String,
    /// 可用配置列表
    pub available_configs: Vec<PathBuf>,
    /// 选中配置索引
    pub selected_config: Option<usize>,
    /// 校验错误
    pub error_message: Option<String>,
    /// 保存路径
    pub config_path: Option<PathBuf>,
    /// 表单状态
    pub form_state: ConfigFormState,
}

impl ConfigWizardState {
    /// 创建向导状态
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否处于创建配置流程
    pub fn is_create_config_flow(&self) -> bool {
        matches!(self.flow, WizardFlow::CreateConfig)
    }

    /// 是否处于直接输入参数运行流程
    pub fn is_run_direct_flow(&self) -> bool {
        matches!(self.flow, WizardFlow::RunDirect)
    }

    /// 是否处于配置选择流程
    pub fn is_select_config_flow(&self) -> bool {
        matches!(self.flow, WizardFlow::RunFromConfig { .. })
    }

    /// 使用已有配置流程中是否需要修改确认
    pub fn needs_modify_confirm(&self) -> bool {
        matches!(
            self.flow,
            WizardFlow::RunFromConfig {
                need_modify_confirm: true
            }
        )
    }

    /// 配置选择是否允许确认
    pub fn can_confirm_config_select(&self) -> bool {
        if self.step == ConfigStep::ConfigSelect {
            !self.available_configs.is_empty()
        } else {
            true
        }
    }

    /// 从配置初始化表单
    pub fn init_from_config(&mut self, config: &Config, config_path: &Path) {
        self.input_dirs = config
            .input_dirs
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ");
        self.output_dir = config.output_dir.display().to_string();
        self.exclude_dirs = config
            .exclude_dirs
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ");
        self.processing_mode.select(config.processing_mode);
        self.classification.select(config.classification);
        self.month_format.select(config.month_format);
        self.operation.select(config.operation);
        self.deduplicate
            .select_by_index(if config.deduplicate { 1 } else { 0 });
        self.dry_run
            .select_by_index(if config.dry_run { 1 } else { 0 });
        self.classify_by_type
            .select_by_index(if config.classify_by_type { 1 } else { 0 });
        self.unify_filenames
            .select_by_index(if config.unify_filenames { 1 } else { 0 });
        self.preserve_standard_names
            .select_by_index(if config.preserve_standard_names { 1 } else { 0 });
        self.config_name = config_path
            .file_stem()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_default();
    }

    /// 构建配置
    pub fn build_config(&self) -> Config {
        let input_dirs: Vec<PathBuf> = self
            .input_dirs
            .split(';')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect();

        let exclude_dirs: Vec<PathBuf> = self
            .exclude_dirs
            .split(';')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect();

        Config {
            input_dirs,
            output_dir: PathBuf::from(&self.output_dir),
            exclude_dirs,
            processing_mode: self.processing_mode.selected(),
            classification: self.classification.selected(),
            month_format: self.month_format.selected(),
            classify_by_type: self.classify_by_type.value(),
            operation: self.operation.selected(),
            deduplicate: self.deduplicate.value(),
            dry_run: self.dry_run.value(),
            unify_filenames: self.unify_filenames.value(),
            preserve_standard_names: self.preserve_standard_names.value(),
            verbose: false,
            ..Default::default()
        }
    }

    /// 校验表单
    pub fn validate_form(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        if !self.is_run_direct_flow() {
            if self.config_name.trim().is_empty() {
                errors.push(rust_i18n::t!("config_name_empty_error").to_string());
            } else if self.config_name.contains('/')
                || self.config_name.contains('\\')
                || self.config_name.contains('.')
            {
                errors.push(rust_i18n::t!("config_name_invalid_chars_error").to_string());
            }
        }

        if self.input_dirs.trim().is_empty() {
            errors.push(rust_i18n::t!("no_input_dirs_specified").to_string());
        }

        if self.output_dir.trim().is_empty() {
            errors.push(rust_i18n::t!("output_dir_empty_error").to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// 从外部扫描结果设置可用配置列表（纯状态更新，IO 由 app.rs 执行）
    pub fn set_configs_from_paths(&mut self, paths: Vec<PathBuf>) {
        self.available_configs = paths;

        if self.available_configs.is_empty() {
            self.selected_config = None;
        } else {
            let max_index = self.available_configs.len().saturating_sub(1);
            let index = self.selected_config.unwrap_or(0).min(max_index);
            self.selected_config = Some(index);
        }
    }

    /// 获取当前选中索引
    pub fn selected_value(&self) -> usize {
        match self.step {
            ConfigStep::ConfirmRun | ConfigStep::ConfigSelect => self.selected_config.unwrap_or(0),
            _ => 0,
        }
    }

    /// 获取当前步骤选项数量
    pub fn option_count(&self) -> usize {
        match self.step {
            ConfigStep::ConfigSelect => self.available_configs.len().max(1),
            ConfigStep::ConfirmRun => 2,
            _ => 0,
        }
    }

    /// 设置选中索引
    pub fn set_selected(&mut self, index: usize) {
        match self.step {
            ConfigStep::ConfirmRun | ConfigStep::ConfigSelect => {
                self.selected_config = Some(index);
            }
            _ => {}
        }
    }

    /// 初始化/校准选择索引
    pub fn ensure_selection(&mut self) {
        match self.step {
            ConfigStep::ConfirmRun => {
                let index = match self.selected_config {
                    Some(index) if index <= 1 => index,
                    _ => 0,
                };
                self.selected_config = Some(index);
            }
            ConfigStep::ConfigSelect => {
                if self.available_configs.is_empty() {
                    self.selected_config = None;
                } else {
                    let max_index = self.available_configs.len() - 1;
                    let index = self.selected_config.unwrap_or(0).min(max_index);
                    self.selected_config = Some(index);
                }
            }
            _ => {}
        }
    }

    /// 选择下一个选项
    pub fn navigate_next(&mut self) {
        match self.step {
            ConfigStep::ConfirmRun | ConfigStep::ConfigSelect => {
                self.ensure_selection();
                if let Some(idx) = self.selected_config {
                    let count = self.option_count();
                    self.selected_config = Some((idx + 1) % count);
                }
            }
            _ => {}
        }
    }

    /// 选择上一个选项
    pub fn navigate_prev(&mut self) {
        match self.step {
            ConfigStep::ConfirmRun | ConfigStep::ConfigSelect => {
                self.ensure_selection();
                if let Some(idx) = self.selected_config {
                    let count = self.option_count();
                    self.selected_config = Some(if idx == 0 { count - 1 } else { idx - 1 });
                }
            }
            _ => {}
        }
    }

    /// 表单选择下一个字段
    pub fn navigate_form_next(&mut self) {
        let visible_count = self.form_state.visible_fields_count(self) + 1;
        if visible_count > 0 {
            self.form_state.next_field(visible_count);
        }
    }

    /// 表单选择上一个字段
    pub fn navigate_form_prev(&mut self) {
        let visible_count = self.form_state.visible_fields_count(self) + 1;
        if visible_count > 0 {
            self.form_state.prev_field(visible_count);
        }
    }

    /// 是否选中“下一步”
    pub fn is_next_selected(&self) -> bool {
        let visible_count = self.form_state.visible_fields_count(self);
        self.form_state.selected_field >= visible_count
    }

    /// 切换当前字段到下一个选项
    pub fn toggle_current_field_next(&mut self) {
        let visible_fields = self.get_visible_fields();
        let field_opt = visible_fields.get(self.form_state.selected_field).copied();

        match field_opt {
            Some(FormField::ProcessingMode) => self.processing_mode.next(),
            Some(FormField::Classification) => self.classification.next(),
            Some(FormField::MonthFormat) => self.month_format.next(),
            Some(FormField::FileOperation) => self.operation.next(),
            Some(FormField::Deduplication) => self.deduplicate.next(),
            Some(FormField::DryRun) => self.dry_run.next(),
            Some(FormField::ClassifyByType) => self.classify_by_type.next(),
            Some(FormField::UnifyFilenames) => self.unify_filenames.next(),
            Some(FormField::PreserveStandardNames) => self.preserve_standard_names.next(),
            _ => {}
        }
    }

    /// 切换当前字段到上一个选项
    pub fn toggle_current_field_prev(&mut self) {
        let visible_fields = self.get_visible_fields();
        let field_opt = visible_fields.get(self.form_state.selected_field).copied();

        match field_opt {
            Some(FormField::ProcessingMode) => self.processing_mode.prev(),
            Some(FormField::Classification) => self.classification.prev(),
            Some(FormField::MonthFormat) => self.month_format.prev(),
            Some(FormField::FileOperation) => self.operation.prev(),
            Some(FormField::Deduplication) => self.deduplicate.prev(),
            Some(FormField::DryRun) => self.dry_run.prev(),
            Some(FormField::ClassifyByType) => self.classify_by_type.prev(),
            Some(FormField::UnifyFilenames) => self.unify_filenames.prev(),
            Some(FormField::PreserveStandardNames) => self.preserve_standard_names.prev(),
            _ => {}
        }
    }

    /// 更新输入字段值
    pub fn update_field_from_input(&mut self, value: String) {
        let visible_fields = self.get_visible_fields();
        let field_opt = visible_fields.get(self.form_state.selected_field).copied();

        match field_opt {
            Some(FormField::ConfigName) => self.config_name = value,
            Some(FormField::InputDirs) => self.input_dirs = value,
            Some(FormField::OutputDir) => self.output_dir = value,
            Some(FormField::ExcludeDirs) => self.exclude_dirs = value,
            _ => {}
        }
    }

    /// 进入输入模式
    pub fn enter_input_mode_for_field(&mut self) {
        let visible_fields = self.get_visible_fields();
        let field_opt = visible_fields.get(self.form_state.selected_field).copied();

        let value = match field_opt {
            Some(FormField::ConfigName) => self.config_name.clone(),
            Some(FormField::InputDirs) => self.input_dirs.clone(),
            Some(FormField::OutputDir) => self.output_dir.clone(),
            Some(FormField::ExcludeDirs) => self.exclude_dirs.clone(),
            _ => return,
        };
        self.form_state.enter_input_mode(&value);
    }

    /// 获取输入内容
    pub fn input_buffer(&self) -> &str {
        self.form_state.input_value()
    }

    /// 获取光标位置
    pub fn input_cursor(&self) -> usize {
        self.form_state.input_cursor()
    }

    /// 设置输入缓冲区
    pub fn set_input_buffer(&mut self, buffer: String, cursor: usize) {
        self.form_state.input.set_buffer(buffer, cursor);
    }

    /// 退出输入模式并应用
    pub fn exit_input_mode_apply(&mut self) {
        let value = self.form_state.input_value().to_string();
        self.update_field_from_input(value);
        self.form_state.exit_input_mode();
    }

    /// 退出输入模式（不保存）
    pub fn exit_input_mode_cancel(&mut self) {
        self.form_state.exit_input_mode();
    }

    /// 获取可见字段
    pub fn get_visible_fields(&self) -> Vec<FormField> {
        FormField::all()
            .iter()
            .filter(|f| f.is_visible(self))
            .copied()
            .collect()
    }

    /// 获取当前可见字段
    pub fn selected_form_field(&self) -> Option<FormField> {
        let visible_fields = self.get_visible_fields();
        visible_fields.get(self.form_state.selected_field).copied()
    }

    /// 是否输入模式
    pub fn is_in_input_mode(&self) -> bool {
        self.form_state.in_input_mode
    }

    /// 输入插入字符
    pub fn input_insert_char(&mut self, c: char) {
        self.form_state.input.insert_char(c);
    }

    /// 输入退格
    pub fn input_backspace(&mut self) {
        self.form_state.input.delete_before_cursor();
    }

    /// 输入删除
    pub fn input_delete(&mut self) {
        self.form_state.input.delete_after_cursor();
    }

    /// 输入光标左移
    pub fn input_move_left(&mut self) {
        self.form_state.input.move_cursor_left();
    }

    /// 输入光标右移
    pub fn input_move_right(&mut self) {
        self.form_state.input.move_cursor_right();
    }

    /// 输入光标到行首
    pub fn input_move_to_start(&mut self) {
        self.form_state.input.move_cursor_to_start();
    }

    /// 输入光标到行尾
    pub fn input_move_to_end(&mut self) {
        self.form_state.input.move_cursor_to_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::Screen;

    /// 三条向导流程（对应 4 个布尔标志的既有组合）
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Flow {
        RunDirect,
        RunFromConfig,
        CreateConfig,
    }

    /// 副作用摘要（只比较种类与配置名有无，不比较完整 Config）
    #[derive(Debug, Clone, PartialEq)]
    enum EffectKind {
        LoadConfig,
        SaveConfig,
        RefreshConfigs,
        RunProcessing { has_name: bool },
    }

    #[derive(Debug)]
    struct Expected {
        step: ConfigStep,
        effects: Vec<EffectKind>,
        resets_main_menu: bool,
    }

    fn state_for(flow: Flow, step: ConfigStep) -> AppState {
        let wizard_flow = match flow {
            Flow::RunDirect => WizardFlow::RunDirect,
            Flow::RunFromConfig => WizardFlow::RunFromConfig {
                need_modify_confirm: true,
            },
            Flow::CreateConfig => WizardFlow::CreateConfig,
        };
        AppState {
            current_screen: Screen::ConfigWizard,
            config_wizard: ConfigWizardState {
                step,
                flow: wizard_flow,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn all_events() -> Vec<TuiEvent> {
        vec![
            TuiEvent::Up,
            TuiEvent::Down,
            TuiEvent::Left,
            TuiEvent::Right,
            TuiEvent::Enter,
            TuiEvent::Escape,
            TuiEvent::Tab,
            TuiEvent::Char('x'),
            TuiEvent::Backspace,
            TuiEvent::Delete,
            TuiEvent::Home,
            TuiEvent::End,
        ]
    }

    fn effect_kinds(effects: &[Effect]) -> Vec<EffectKind> {
        effects
            .iter()
            .map(|effect| match effect {
                Effect::LoadConfig(_) => EffectKind::LoadConfig,
                Effect::SaveConfig { .. } => EffectKind::SaveConfig,
                Effect::RefreshConfigs => EffectKind::RefreshConfigs,
                Effect::RunProcessing { config_name, .. } => EffectKind::RunProcessing {
                    has_name: config_name.is_some(),
                },
            })
            .collect()
    }

    /// 转换表：给定（flow 标志组, step, event），断言（step, 副作用, 是否重置主菜单）
    fn expected_for(flow: Flow, step: ConfigStep, event: TuiEvent) -> Expected {
        let inert = Expected {
            step: step.clone(),
            effects: Vec::new(),
            resets_main_menu: false,
        };

        match (&step, event) {
            (ConfigStep::ConfigSelect, TuiEvent::Escape) => Expected {
                step: ConfigStep::ConfigSelect,
                effects: vec![],
                resets_main_menu: true,
            },
            (ConfigStep::ConfigSelect, _) => inert,
            (ConfigStep::ConfigForm, TuiEvent::Enter) => {
                // 默认选中第一个可见字段（输入字段）→ 进入输入模式
                Expected {
                    step: ConfigStep::ConfigForm,
                    effects: vec![],
                    resets_main_menu: false,
                }
            }
            (ConfigStep::ConfigForm, TuiEvent::Escape) => match flow {
                Flow::RunFromConfig => Expected {
                    step: ConfigStep::ConfigSelect,
                    effects: vec![],
                    resets_main_menu: false,
                },
                _ => Expected {
                    step: ConfigStep::ConfigSelect,
                    effects: vec![],
                    resets_main_menu: true,
                },
            },
            (ConfigStep::ConfigForm, _) => inert,
            (ConfigStep::Summary, TuiEvent::Enter) => match flow {
                Flow::RunDirect => Expected {
                    step: ConfigStep::Summary,
                    effects: vec![EffectKind::RunProcessing { has_name: false }],
                    resets_main_menu: false,
                },
                Flow::RunFromConfig | Flow::CreateConfig => Expected {
                    step: ConfigStep::ConfirmRun,
                    effects: vec![],
                    resets_main_menu: false,
                },
            },
            (ConfigStep::Summary, TuiEvent::Escape) => match flow {
                Flow::RunFromConfig => Expected {
                    step: ConfigStep::ConfigSelect,
                    effects: vec![],
                    resets_main_menu: false,
                },
                _ => Expected {
                    step: ConfigStep::ConfigForm,
                    effects: vec![],
                    resets_main_menu: false,
                },
            },
            (ConfigStep::Summary, _) => inert,
            (ConfigStep::ConfirmRun, TuiEvent::Enter) => match flow {
                Flow::RunFromConfig => Expected {
                    step: ConfigStep::ConfigForm,
                    effects: vec![],
                    resets_main_menu: false,
                },
                Flow::RunDirect | Flow::CreateConfig => Expected {
                    step: ConfigStep::ConfirmRun,
                    effects: vec![
                        EffectKind::SaveConfig,
                        EffectKind::RunProcessing { has_name: true },
                    ],
                    resets_main_menu: false,
                },
            },
            (ConfigStep::ConfirmRun, TuiEvent::Escape) => Expected {
                step: ConfigStep::Summary,
                effects: vec![],
                resets_main_menu: false,
            },
            (ConfigStep::ConfirmRun, _) => inert,
        }
    }

    #[test]
    fn test_transition_table_all_flow_step_event_combinations() {
        let flows = [Flow::RunDirect, Flow::RunFromConfig, Flow::CreateConfig];
        let steps = [
            ConfigStep::ConfigSelect,
            ConfigStep::ConfigForm,
            ConfigStep::Summary,
            ConfigStep::ConfirmRun,
        ];

        for flow in flows {
            for step in steps.clone() {
                for event in all_events() {
                    let mut state = state_for(flow, step.clone());
                    let effects = transition(&mut state, event.clone());
                    let expected = expected_for(flow, step.clone(), event.clone());

                    assert_eq!(
                        state.config_wizard.step, expected.step,
                        "flow={flow:?} step={step:?} event={event:?}"
                    );
                    assert_eq!(
                        effect_kinds(&effects),
                        expected.effects,
                        "flow={flow:?} step={step:?} event={event:?}"
                    );
                    assert_eq!(
                        state.current_screen == Screen::MainMenu,
                        expected.resets_main_menu,
                        "flow={flow:?} step={step:?} event={event:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_config_select_enter_emits_load_config_effect() {
        let mut state = state_for(Flow::RunFromConfig, ConfigStep::ConfigSelect);
        state.config_wizard.available_configs = vec![PathBuf::from("C:/configs/album.toml")];

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(effect_kinds(&effects), vec![EffectKind::LoadConfig]);
        assert_eq!(state.config_wizard.step, ConfigStep::ConfigSelect);
    }

    #[test]
    fn test_run_from_config_summary_modified_confirm_runs_directly() {
        // 修改确认已完成 → Summary Enter 直接运行
        let mut state = state_for(Flow::RunFromConfig, ConfigStep::Summary);
        state.config_wizard.flow = WizardFlow::RunFromConfig {
            need_modify_confirm: false,
        };
        state.config_wizard.config_name = "album".to_string();

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(
            effect_kinds(&effects),
            vec![EffectKind::RunProcessing { has_name: true }]
        );
        assert_eq!(state.config_wizard.step, ConfigStep::Summary);
    }

    #[test]
    fn test_run_from_config_confirm_run_selected_run_emits_effect() {
        let mut state = state_for(Flow::RunFromConfig, ConfigStep::ConfirmRun);
        state.config_wizard.config_name = "album".to_string();
        state.config_wizard.set_selected(1);

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(
            effect_kinds(&effects),
            vec![EffectKind::RunProcessing { has_name: true }]
        );
        assert_eq!(state.config_wizard.step, ConfigStep::ConfirmRun);
    }

    #[test]
    fn test_create_config_confirm_run_decline_resets_to_main_menu() {
        let mut state = state_for(Flow::CreateConfig, ConfigStep::ConfirmRun);
        state.config_wizard.config_name = "album".to_string();
        state.config_wizard.set_selected(1);

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(effect_kinds(&effects), vec![EffectKind::SaveConfig]);
        assert_eq!(state.current_screen, Screen::MainMenu);
    }

    #[test]
    fn test_create_config_confirm_run_decline_saves_config_snapshot() {
        let mut state = state_for(Flow::CreateConfig, ConfigStep::ConfirmRun);
        state.config_wizard.config_name = "album".to_string();
        state.config_wizard.input_dirs = "D:/photos".to_string();
        state.config_wizard.output_dir = "D:/sorted".to_string();
        state.config_wizard.set_selected(1);

        let effects = transition(&mut state, TuiEvent::Enter);

        // reset 会清空向导状态，但 SaveConfig 效果必须携带填写时的配置快照
        assert_eq!(state.current_screen, Screen::MainMenu);
        match &effects[0] {
            Effect::SaveConfig { name, config } => {
                assert_eq!(name, "album");
                assert_eq!(config.input_dirs, vec![PathBuf::from("D:/photos")]);
                assert_eq!(config.output_dir, PathBuf::from("D:/sorted"));
            }
            other => panic!("expected SaveConfig effect, got {other:?}"),
        }
    }

    #[test]
    fn test_config_form_next_validates_and_advances_to_summary() {
        let mut state = state_for(Flow::CreateConfig, ConfigStep::ConfigForm);
        state.config_wizard.config_name = "album".to_string();
        state.config_wizard.input_dirs = "D:/photos".to_string();
        state.config_wizard.output_dir = "D:/sorted".to_string();
        // 将选中索引移到“下一步”伪字段（unify 关闭时可见字段 12 个 → 索引 12）
        state.config_wizard.form_state.selected_field = 12;
        assert!(state.config_wizard.is_next_selected());

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(state.config_wizard.step, ConfigStep::Summary);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_config_form_next_invalid_sets_error_message() {
        let mut state = state_for(Flow::CreateConfig, ConfigStep::ConfigForm);
        state.config_wizard.form_state.selected_field = 12;

        let effects = transition(&mut state, TuiEvent::Enter);

        assert_eq!(state.config_wizard.step, ConfigStep::ConfigForm);
        assert!(state.config_wizard.error_message.is_some());
        assert!(effects.is_empty());
    }

    #[test]
    fn test_preserve_standard_names_visible_only_when_unify_enabled() {
        let mut state = state_for(Flow::CreateConfig, ConfigStep::ConfigForm);

        // 未开启统一文件名：字段隐藏
        assert!(!FormField::PreserveStandardNames.is_visible(&state.config_wizard));

        // 开启统一文件名：字段显示
        state.config_wizard.unify_filenames.select_by_index(1);
        assert!(FormField::PreserveStandardNames.is_visible(&state.config_wizard));
    }

    #[test]
    fn test_config_form_input_mode_interactions() {
        let mut state = state_for(Flow::RunDirect, ConfigStep::ConfigForm);

        // Enter 进入输入模式
        transition(&mut state, TuiEvent::Enter);
        assert!(state.config_wizard.is_in_input_mode());

        transition(&mut state, TuiEvent::Char('h'));
        transition(&mut state, TuiEvent::Char('i'));
        assert_eq!(state.config_wizard.input_buffer(), "hi");
        assert_eq!(state.config_wizard.input_cursor(), 2);

        transition(&mut state, TuiEvent::Home);
        assert_eq!(state.config_wizard.input_cursor(), 0);
        transition(&mut state, TuiEvent::End);
        assert_eq!(state.config_wizard.input_cursor(), 2);

        transition(&mut state, TuiEvent::Backspace);
        assert_eq!(state.config_wizard.input_buffer(), "h");
        assert_eq!(state.config_wizard.input_cursor(), 1);

        transition(&mut state, TuiEvent::Char('e'));
        assert_eq!(state.config_wizard.input_buffer(), "he");
        transition(&mut state, TuiEvent::Left);
        assert_eq!(state.config_wizard.input_cursor(), 1);
        transition(&mut state, TuiEvent::Delete);
        assert_eq!(state.config_wizard.input_buffer(), "h");

        // Escape 取消输入，不应用
        transition(&mut state, TuiEvent::Escape);
        assert!(!state.config_wizard.is_in_input_mode());
        assert_eq!(state.config_wizard.input_dirs, "");

        // 再次进入并应用
        transition(&mut state, TuiEvent::Enter);
        assert!(state.config_wizard.is_in_input_mode());
        transition(&mut state, TuiEvent::Char('D'));
        transition(&mut state, TuiEvent::Enter);
        assert!(!state.config_wizard.is_in_input_mode());
        assert_eq!(state.config_wizard.input_dirs, "D");
    }
}
