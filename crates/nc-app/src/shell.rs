//! The compact read-only desktop shell.
//!
//! One workspace, two reserved modes, and three sheets: file open, move
//! legend, and settings. The viewer dominates the window. There is no
//! controller panel, DRO, jog, MDI, console, connect, reset, or start control;
//! the only file mutation is reading a local program into memory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, ExternalPaths, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, ScrollStrategy,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use nc_gcode::Program;
use nc_viewer::{MoveFamily, MoveKind, SegmentKind, StockBox};

use crate::file_browser::{
    BrowserEntry, FileBrowser, compact_path, default_root, display_path, format_size, read_program,
};
use crate::layout::{STATUS_BAR_HEIGHT, ShellLayout, TOUCH_TARGET};
use crate::program::SourceLine;
use crate::state::{AppMode, LoadedProgram, NcaConfig, NcaState, ParseState};
use crate::theme::{Palette, ThemeMode, apply as apply_theme, move_color, move_style};
use crate::viewer::{self, HasViewer, ViewerHandle, ViewerMode};

/// The shell view.
pub struct NcaShell {
    state: NcaState,
    viewer: ViewerHandle,
    mono: SharedString,
    program_scroll: UniformListScrollHandle,
    program_search_input: Entity<InputState>,
    browser_path_input: Entity<InputState>,
    browser: Option<FileBrowser>,
    legend_open: bool,
    load_error: Option<String>,
    pending_search_reset: bool,
    _load_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl NcaShell {
    /// Builds the shell and loads the built-in demo.
    pub fn new(
        window: &mut Window,
        open_path: Option<PathBuf>,
        view_3d: bool,
        open_browser: bool,
        initial_theme: ThemeMode,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = NcaState::new(NcaConfig::load());
        // A launch flag overrides the persisted mode for this run without
        // rewriting app.json; the settings selector persists explicit changes.
        state.config.theme = initial_theme;
        apply_theme(initial_theme, cx);

        let program_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.search.clone())
                .placeholder("Text, N120, or line 120")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let browser_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/path/to/folder")
                .clean_on_escape()
                .submit_on_enter(true)
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(
            &program_search_input,
            |this, input, event, cx| match event {
                InputEvent::Change => {
                    this.state.search = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.select_first_match(cx);
                }
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(&browser_path_input, |this, input, event, cx| {
            if let InputEvent::Change = event {
                let draft = input.read(cx).value().to_string();
                if let Some(browser) = this.browser.as_mut() {
                    browser.set_path_draft(draft);
                }
            }
            if let InputEvent::PressEnter { .. } = event {
                this.navigate_browser(cx);
            }
        }));

        let mono = cx.theme().mono_font_family.clone();
        let mut shell = Self {
            state,
            viewer: ViewerHandle::new(),
            mono,
            program_scroll: UniformListScrollHandle::new(),
            program_search_input,
            browser_path_input,
            browser: None,
            legend_open: false,
            load_error: None,
            pending_search_reset: false,
            _load_task: None,
            _subscriptions: subscriptions,
        };
        if let Some(path) = open_path {
            shell.load_initial(path);
        }
        if view_3d {
            shell.viewer.set_mode(ViewerMode::ThreeD);
        }
        if open_browser {
            shell.open_browser(window, cx);
        }
        shell
    }

    fn load_initial(&mut self, path: PathBuf) {
        match read_program(&path) {
            Ok(file) => {
                self.state.adopt(LoadedProgram::from_file(file));
                self.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.viewer.fit();
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
            }
        }
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.load_error = None;
        self.state.loading = true;
        cx.notify();
        let task_path = path.clone();
        self._load_task = Some(cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match read_program(&task_path) {
                        Ok(file) => Ok(LoadedProgram::from_file(file)),
                        Err(error) => Err(error.to_string()),
                    }
                })
                .await;
            let _ = weak.update(cx, |shell, cx| {
                match result {
                    Ok(loaded) => {
                        shell.state.adopt(loaded);
                        shell.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        shell.browser = None;
                        shell.load_error = None;
                        shell.pending_search_reset = true;
                        shell.viewer.fit();
                    }
                    Err(message) => {
                        shell.state.loading = false;
                        shell.load_error = Some(message);
                        if let Some(browser) = shell.browser.as_mut() {
                            browser.set_error("Could not load the selected file");
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    fn open_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .state
            .config
            .last_dir
            .clone()
            .or_else(|| {
                self.state
                    .loaded
                    .as_ref()
                    .and_then(|loaded| loaded.path.as_deref())
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(default_root);
        self.browser = Some(FileBrowser::open(&start));
        self.sync_browser_input(window, cx);
        cx.notify();
    }

    fn sync_browser_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .browser
            .as_ref()
            .map_or_else(String::new, FileBrowser::path_display);
        self.browser_path_input
            .update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn navigate_browser(&mut self, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        if !browser.commit_path() {
            cx.notify();
            return;
        }
        cx.notify();
    }

    fn browser_click(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        let is_dir = browser
            .entries()
            .get(index)
            .is_some_and(|entry| entry.is_dir);
        if is_dir {
            if browser.enter(index) {
                self.sync_browser_input(window, cx);
            }
        } else {
            browser.select(index);
        }
        cx.notify();
    }

    fn browser_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        if browser.up() {
            self.sync_browser_input(window, cx);
        }
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let path = self
            .browser
            .as_ref()
            .and_then(FileBrowser::selected_entry)
            .map(|entry| entry.path.clone());
        if let Some(path) = path {
            self.open_path(path, cx);
        }
    }

    fn close_browser(&mut self, cx: &mut Context<Self>) {
        self.browser = None;
        cx.notify();
    }

    fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.state.config.set_theme(mode);
        apply_theme(mode, cx);
        cx.notify();
    }

    fn select_first_match(&mut self, cx: &mut Context<Self>) {
        self.state.ensure_index();
        let Some(loaded) = self.state.loaded.as_ref() else {
            return;
        };
        let index = self
            .state
            .index()
            .cloned()
            .unwrap_or_else(|| loaded.index(&self.state.search));
        let Some(source) = index.first_match() else {
            return;
        };
        let Some(line) = loaded.lines.get(source) else {
            return;
        };
        self.state.selected_line = Some(line.number);
        if let Some(visible) = index.visible_index(source) {
            self.program_scroll
                .scroll_to_item(visible, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.search.clear();
        self.program_search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn drop_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        match paths {
            [path] if path.is_file() => self.open_path(path.clone(), cx),
            [] => {
                self.load_error =
                    Some("Drop one .nc, .gcode, .tap, or .ngc program file".to_string());
                cx.notify();
            }
            [path] => {
                self.load_error = Some(format!("'{}' is not a regular file", display_path(path)));
                cx.notify();
            }
            _ => {
                self.load_error = Some("Drop exactly one program file".to_string());
                cx.notify();
            }
        }
    }

    fn top_bar(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let target = layout.touch_target();
        let mode_chip = |label: &'static str, active: bool, available: bool| {
            let mut chip = div()
                .id(if active { "mode-view" } else { "mode-simulate" })
                .h(px(target))
                .min_w(px(84.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .border_1()
                .border_color(if active { p.accent } else { p.border })
                .bg(if active { p.accent_muted } else { p.panel_alt })
                .text_size(px(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(if active { p.accent } else { p.text_dim })
                .child(label);
            if !available {
                let hint = format!("{label} arrives in the simulation milestone");
                chip = chip
                    .opacity(0.55)
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx));
            }
            chip
        };

        let brand = h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(px(21.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.accent)
                    .child("NCA"),
            )
            .child(
                v_flex()
                    .gap_0()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_dim)
                            .child("READ-ONLY NC VIEWER"),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .text_color(p.text_faint)
                            .child("3-AXIS MILL | LOCAL ONLY"),
                    ),
            );

        let open = action_button(
            "bar-open",
            "OPEN",
            true,
            false,
            p,
            view.clone(),
            move |shell, window, cx| {
                shell.open_browser(window, cx);
            },
        );

        let fit = action_button(
            "bar-fit",
            "FIT",
            true,
            false,
            p,
            view.clone(),
            move |shell, _window, cx| {
                shell.viewer.fit();
                cx.notify();
            },
        );

        let mode_2d = self.viewer.mode() == ViewerMode::Top2D;
        let two_d = action_button(
            "bar-2d",
            "2D",
            true,
            mode_2d,
            p,
            view.clone(),
            move |shell, _, cx| {
                if shell.viewer.set_mode(ViewerMode::Top2D) {
                    cx.notify();
                }
            },
        );
        let mode_3d = self.viewer.mode() == ViewerMode::ThreeD;
        let three_d = action_button(
            "bar-3d",
            "3D",
            true,
            mode_3d,
            p,
            view.clone(),
            move |shell, _, cx| {
                if shell.viewer.set_mode(ViewerMode::ThreeD) {
                    cx.notify();
                }
            },
        );

        let stock_on = self.viewer.stock_visible();
        let stock = action_button(
            "bar-stock",
            if stock_on { "STOCK ON" } else { "STOCK OFF" },
            true,
            stock_on,
            p,
            view.clone(),
            move |shell, _, cx| {
                let next = !shell.viewer.stock_visible();
                if shell.viewer.set_stock_visible(next) {
                    shell.state.stock_visible = next;
                    cx.notify();
                }
            },
        );

        let settings = action_button(
            "bar-settings",
            "SETTINGS",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| {
                shell.state.settings_open = true;
                cx.notify();
            },
        );

        let analyze = placeholder_button("bar-analyze", "ANALYZE", p);
        let export = placeholder_button("bar-export", "EXPORT", p);

        let actions = h_flex()
            .flex_1()
            .min_w(px(0.0))
            .justify_end()
            .items_center()
            .gap_1()
            .child(open)
            .child(fit)
            .child(two_d)
            .child(three_d)
            .child(stock)
            .child(analyze)
            .child(export)
            .child(settings);

        if layout.top_bar_rows() == 2 {
            return v_flex()
                .flex_shrink_0()
                .gap_1()
                .p_1()
                .border_b_1()
                .border_color(p.border)
                .bg(p.panel)
                .child(
                    h_flex()
                        .h(px(target))
                        .items_center()
                        .gap_2()
                        .child(brand)
                        .child(div().flex_1())
                        .child(mode_chip(AppMode::View.label(), true, true))
                        .child(mode_chip(AppMode::Simulate.label(), false, false)),
                )
                .child(h_flex().h(px(target)).items_center().gap_1().child(actions))
                .into_any_element();
        }

        h_flex()
            .h(px(target + 8.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(brand)
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(mode_chip(AppMode::View.label(), true, true))
                    .child(mode_chip(AppMode::Simulate.label(), false, false)),
            )
            .child(actions)
            .into_any_element()
    }

    fn status_strip(&self, p: Palette) -> AnyElement {
        let loaded = self.state.loaded.as_ref();
        let mut strip = h_flex()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel_alt);

        let name = self.state.file_name().to_string();
        let full_name = loaded
            .and_then(|loaded| loaded.path.as_deref())
            .map_or_else(|| name.clone(), display_path);
        strip = strip.child(
            div()
                .id("status-file")
                .h(px(22.0))
                .max_w(px(260.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .overflow_hidden()
                .rounded(px(5.0))
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .px_2()
                .text_size(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text)
                .truncate()
                .tooltip(move |window, cx| Tooltip::new(full_name.clone()).build(window, cx))
                .child(name),
        );

        strip = strip.child(chip(&format!("{} LINES", self.state.line_count()), p));
        let tools = loaded.map_or(0, LoadedProgram::tool_count);
        let tools_text = if tools == 1 {
            "1 TOOL".to_string()
        } else {
            format!("{tools} TOOLS")
        };
        strip = strip.child(chip(&tools_text, p));

        if let Some(loaded) = loaded {
            let stock_label = loaded.stock_label();
            let tip = format!("{}\n{}", loaded.stock_size_text(), loaded.stock_basis());
            strip = strip.child(
                chip(stock_label, p)
                    .id("status-stock")
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
            );
        }

        let parse = self.state.parse_state();
        let parse_color = match parse {
            ParseState::ParseErrors(_) => p.danger,
            ParseState::Loading => p.info,
            _ => p.text_dim,
        };
        let parse_border = if matches!(parse, ParseState::ParseErrors(_)) {
            p.danger
        } else {
            p.border
        };
        strip = strip.child(
            chip(&parse.label(), p)
                .text_color(parse_color)
                .border_color(parse_border),
        );

        if let Some(line) = self.state.selected_line {
            strip = strip.child(chip(&format!("L{line}"), p));
        }

        strip = strip.child(div().flex_1());
        strip = strip.child(
            div()
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child("ADVISORY ONLY | NO CONTROLLER"),
        );
        strip.into_any_element()
    }

    fn workspace(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        match layout.columns {
            crate::layout::ColumnLayout::Three => h_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .child(self.program_panel(p, view.clone(), Some(layout.left_width), None))
                .child(self.canvas_card(layout, p, view.clone()))
                .child(self.legend_panel(layout, p, view))
                .into_any_element(),
            crate::layout::ColumnLayout::Two => h_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .child(self.program_panel(p, view.clone(), Some(layout.left_width), None))
                .child(self.canvas_card(layout, p, view))
                .into_any_element(),
            crate::layout::ColumnLayout::One => v_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .overflow_y_scrollbar()
                .child(self.canvas_card_portrait(layout, p, view.clone()))
                .child(self.collapsed_legend(p, view.clone()))
                .child(self.program_panel(p, view, None, Some(portrait_program_height(layout))))
                .into_any_element(),
        }
    }

    fn program_panel(
        &self,
        p: Palette,
        view: Entity<Self>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> AnyElement {
        let Some(loaded) = self.state.loaded.as_ref() else {
            let mut panel = v_flex()
                .rounded_lg()
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .p_3()
                .gap_2()
                .child(section_label("PROGRAM", p))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(p.text_faint)
                        .child("No program is open. Use OPEN to choose a local file."),
                );
            if let Some(width) = width {
                panel = panel.w(px(width)).h_full();
            }
            if let Some(height) = height {
                panel = panel.w_full().h(px(height));
            }
            return panel.into_any_element();
        };

        let index = self
            .state
            .index()
            .cloned()
            .unwrap_or_else(|| loaded.index(&self.state.search));
        let visible_count = index.visible_count();
        let line_count = loaded.line_count();
        let selected = self.state.selected_line;
        let palette = p;
        let list_view = view.clone();
        let row_lines = Arc::clone(&loaded.lines);
        let mono = self.mono.clone();
        let row_index = index.clone();

        let body = if visible_count == 0 {
            v_flex()
                .flex_1()
                .min_h(px(0.0))
                .items_center()
                .justify_center()
                .p_3()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child("NO MATCHING LINES"),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(p.text_faint)
                        .child("Try text, N120, or line 120."),
                )
                .into_any_element()
        } else {
            uniform_list("program-list", visible_count, move |range, _, _| {
                range
                    .filter_map(|visible| {
                        let source = row_index.source_index(visible)?;
                        let line = row_lines.get(source)?;
                        Some(
                            program_row(
                                line,
                                row_index.has_error(line.number),
                                selected == Some(line.number),
                                palette,
                                mono.clone(),
                                list_view.clone(),
                            )
                            .into_any_element(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(&self.program_scroll)
            .into_any_element()
        };

        let header = h_flex()
            .h(px(32.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("PROGRAM", p))
            .child(div().flex_1())
            .child(chip(&format!("{line_count} LINES"), p))
            .child(if index.is_active() {
                chip(&format!("{visible_count} MATCH"), p).into_any_element()
            } else {
                chip("ALL", p).into_any_element()
            });

        let clear_view = view.clone();
        let search_row = h_flex()
            .h(px(TOUCH_TARGET + 8.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel_alt)
                    .on_mouse_down(MouseButton::Left, {
                        let input = self.program_search_input.clone();
                        move |_, window, cx| {
                            input.update(cx, |state, cx| state.focus(window, cx));
                        }
                    })
                    .child(
                        Input::new(&self.program_search_input)
                            .h(px(TOUCH_TARGET - 2.0))
                            .w_full()
                            .text_size(px(12.0)),
                    ),
            )
            .child(
                div()
                    .id("program-search-clear")
                    .h(px(TOUCH_TARGET))
                    .min_w(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if index.is_active() {
                        p.text
                    } else {
                        p.text_faint
                    })
                    .hover(|style| style.bg(p.raised_hover).text_color(p.text))
                    .on_click(move |_, window, cx| {
                        clear_view.update(cx, |shell, cx| shell.clear_search(window, cx));
                    })
                    .child("CLEAR"),
            );

        let list_header = h_flex()
            .h(px(22.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(div().w(px(18.0)))
            .child(
                div()
                    .w(px(48.0))
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("LINE"),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("N"),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("SOURCE"),
            );

        let mut panel = v_flex()
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(header)
            .child(search_row)
            .child(list_header)
            .child(body);
        if let Some(width) = width {
            panel = panel.w(px(width)).h_full();
        }
        if let Some(height) = height {
            panel = panel.w_full().h(px(height));
        }
        panel.into_any_element()
    }

    fn canvas_card(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let card = div()
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .h_full()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel);
        self.canvas_contents(card, p, view, layout.legend_inline())
    }

    fn canvas_card_portrait(
        &self,
        layout: ShellLayout,
        p: Palette,
        view: Entity<Self>,
    ) -> AnyElement {
        let card = div()
            .relative()
            .w_full()
            .h(px(layout.viewer_height))
            .flex_shrink_0()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel);
        self.canvas_contents(card, p, view, false)
    }

    fn canvas_contents(
        &self,
        mut card: Div,
        p: Palette,
        view: Entity<Self>,
        legend_inline: bool,
    ) -> AnyElement {
        let loaded = self.state.loaded.as_ref();
        let program: Option<Arc<Program>> = loaded.and_then(|loaded| loaded.program.clone());
        let stock: Option<StockBox> = loaded
            .and_then(|loaded| loaded.stock)
            .and_then(|stock| stock.to_stock_box());
        let marker = viewer::program_start_marker(program.as_deref());
        let visibility = SegmentKind::ALL.map(|kind| self.state.segment_visible(kind));
        let mode = self.viewer.mode();
        let zoom = self.viewer.zoom_percent();
        let parse_errors = loaded.is_some_and(|loaded| !loaded.errors.is_empty());
        let preview = loaded.is_none();

        card = card.child(viewer::view(
            &self.viewer,
            p,
            program,
            stock,
            marker,
            visibility,
            preview,
            view.clone(),
        ));

        let mut top_left = h_flex()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .gap_1()
            .child(chip(mode.label(), p))
            .child(chip(&format!("{zoom}%"), p));
        if parse_errors {
            top_left = top_left.child(
                chip("GEOMETRY UNAVAILABLE", p)
                    .text_color(p.danger)
                    .border_color(p.danger),
            );
        }
        card = card.child(top_left);

        card = card.child(
            h_flex()
                .absolute()
                .top(px(8.0))
                .right(px(8.0))
                .gap_1()
                .child(chip("READ-ONLY", p)),
        );

        let hint = if mode.is_three_d() {
            "DRAG ORBIT | SHIFT+DRAG PAN | WHEEL ZOOM | DOUBLE-CLICK FIT"
        } else {
            "DRAG PAN | WHEEL ZOOM | DOUBLE-CLICK FIT"
        };
        card = card.child(
            div()
                .absolute()
                .bottom(px(8.0))
                .left(px(10.0))
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child(hint),
        );

        if legend_inline {
            return card.into_any_element();
        }
        let legend_view = view;
        card.child(
            div()
                .id("legend-collapsed-button")
                .absolute()
                .bottom(px(8.0))
                .right(px(8.0))
                .h(px(TOUCH_TARGET))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .rounded_lg()
                .border_1()
                .border_color(p.border_strong)
                .bg(p.panel)
                .cursor_pointer()
                .hover(|style| style.bg(p.raised_hover))
                .on_click(move |_, _, cx| {
                    legend_view.update(cx, |shell, cx| {
                        shell.legend_open = true;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child("LEGEND"),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.accent)
                        .child("OPEN"),
                ),
        )
        .into_any_element()
    }

    fn legend_panel(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let reset_view = view.clone();
        let header = h_flex()
            .h(px(32.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("MOVE LEGEND", p))
            .child(div().flex_1())
            .child(chip(&format!("{} KINDS", MoveKind::ALL.len()), p))
            .child(
                div()
                    .id("legend-all")
                    .h(px(22.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover).text_color(p.text))
                    .on_click(move |_, _, cx| {
                        reset_view.update(cx, |shell, cx| {
                            shell.state.show_all_kinds();
                            cx.notify();
                        });
                    })
                    .child("ALL"),
            );

        v_flex()
            .w(px(layout.right_width))
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(self.legend_rows(p, view)),
            )
            .into_any_element()
    }

    fn legend_rows(&self, p: Palette, view: Entity<Self>) -> Div {
        let mut column = v_flex().gap_1().p_2();
        for (family, kinds) in legend_groups() {
            let floor = div()
                .h(px(20.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(family_color(family, p)),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_faint)
                        .child(family.label().to_ascii_uppercase()),
                );
            column = column.child(floor);
            for kind in kinds {
                column = column.child(self.legend_row(kind, p, view.clone()));
            }
        }
        column.child(cut_depth_ramp(p))
    }

    fn legend_row(&self, kind: MoveKind, p: Palette, view: Entity<Self>) -> Stateful<Div> {
        let visible = self.state.kind_visible(kind);
        let style = move_style(kind);
        let color = move_color(kind, p);
        let id = format!("legend-{kind:?}");
        div()
            .id(id)
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(if visible { p.border } else { p.panel })
            .bg(if visible { p.panel_alt } else { p.panel })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, _, cx| {
                view.update(cx, |shell, cx| {
                    shell.state.toggle_kind(kind);
                    cx.notify();
                });
            })
            .child(legend_swatch(style, color))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.5))
                    .font_weight(if visible {
                        FontWeight::NORMAL
                    } else {
                        FontWeight::BOLD
                    })
                    .text_color(if visible { p.text } else { p.text_faint })
                    .child(style.label),
            )
            .child(
                div()
                    .w(px(22.0))
                    .text_size(px(8.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if visible { p.accent } else { p.text_faint })
                    .child(if visible { "ON" } else { "OFF" }),
            )
    }

    fn collapsed_legend(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let mut bar = h_flex()
            .id("legend-collapsed")
            .w_full()
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .px_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, _, cx| {
                view.update(cx, |shell, cx| {
                    shell.legend_open = true;
                    cx.notify();
                });
            })
            .child(section_label("MOVE LEGEND", p))
            .child(chip(
                &format!(
                    "{} / {}",
                    self.state.visible_kind_count(),
                    MoveKind::ALL.len()
                ),
                p,
            ));
        for family in MoveFamily::ALL {
            bar = bar.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(family_color(family, p)),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .child(family.label().to_ascii_uppercase()),
                    ),
            );
        }
        bar.child(div().flex_1())
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.accent)
                    .child("OPEN"),
            )
            .into_any_element()
    }

    fn legend_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(620.0))
                    .max_w_full()
                    .h_full()
                    .max_h(px(680.0))
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(
                        h_flex()
                            .h(px(52.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(section_label("MOVE LEGEND", p))
                            .child(chip(&format!("{} KINDS", MoveKind::ALL.len()), p))
                            .child(div().flex_1())
                            .child(action_button(
                                "legend-sheet-close",
                                "CLOSE",
                                true,
                                false,
                                p,
                                view.clone(),
                                move |shell, _, cx| {
                                    shell.legend_open = false;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scrollbar()
                            .child(self.legend_rows(p, view)),
                    ),
            )
            .into_any_element()
    }

    fn settings_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let mut theme_row = h_flex().gap_2().items_center();
        for mode in ThemeMode::ALL {
            let active = self.state.config.theme == mode;
            theme_row = theme_row.child(action_button(
                theme_button_id(mode),
                mode.label(),
                true,
                active,
                p,
                view.clone(),
                move |shell, _, cx| shell.set_theme(mode, cx),
            ));
        }
        let placeholder = |title: &'static str, detail: &'static str| {
            v_flex()
                .gap_1()
                .p_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(p.border)
                .bg(p.panel_alt)
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.text_faint)
                        .child(detail),
                )
        };
        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(620.0))
                    .max_w_full()
                    .max_h_full()
                    .overflow_y_scrollbar()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(section_label("SETTINGS", p))
                            .child(chip("LOCAL ONLY", p))
                            .child(div().flex_1())
                            .child(action_button(
                                "settings-close",
                                "CLOSE",
                                true,
                                false,
                                p,
                                view.clone(),
                                move |shell, _, cx| {
                                    shell.state.settings_open = false;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_label("APPEARANCE", p))
                            .child(theme_row),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_label("RESERVED FOR THE ANALYSIS MILESTONE", p))
                            .child(placeholder(
                                "STOCK AND TOOLS",
                                "Explicit stock and tools arrive with the setup sheet.",
                            ))
                            .child(placeholder(
                                "RULES AND COVERAGE",
                                "Rule counts and skipped reasons arrive with analysis.",
                            ))
                            .child(placeholder(
                                "MACHINE AND MATERIAL",
                                "Machine limits and presets arrive with the simulator.",
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(p.text_faint)
                            .child("Never writes G-code or connects to a controller."),
                    ),
            )
            .into_any_element()
    }

    fn browser_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(browser) = self.browser.as_ref() else {
            return div().into_any_element();
        };
        let entries = browser.entries_arc();
        let selected = browser.selected_index();
        let path_text = browser.path_display();
        let error = browser.error().map(str::to_string);
        let entry_count = entries.len();
        let program_count = browser.file_count();
        let folder = browser
            .cwd()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| browser.path_display());
        let recent = self.state.config.recent.clone();
        let selected_name = browser.selected_entry().map_or_else(
            || "No file selected".to_string(),
            |entry| entry.name.clone(),
        );

        let list_view = view.clone();
        let recent_view = view.clone();

        let header = h_flex()
            .h(px(52.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("OPEN PROGRAM", p))
            .child(chip("LOCAL ONLY", p))
            .child(chip(&format!("{entry_count} ITEMS"), p))
            .child(chip(&format!("{program_count} PROGRAMS"), p))
            .child(chip(&folder, p))
            .child(div().flex_1())
            .child(action_button(
                "browser-close",
                "CLOSE",
                true,
                false,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.close_browser(cx);
                },
            ));

        let path_row = h_flex()
            .h(px(TOUCH_TARGET + 12.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(action_button(
                "browser-up",
                "UP",
                true,
                false,
                p,
                view.clone(),
                move |shell, window, cx| {
                    shell.browser_up(window, cx);
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .on_mouse_down(MouseButton::Left, {
                        let input = self.browser_path_input.clone();
                        move |_, window, cx| {
                            input.update(cx, |state, cx| state.focus(window, cx));
                        }
                    })
                    .child(
                        Input::new(&self.browser_path_input)
                            .h(px(TOUCH_TARGET))
                            .w_full()
                            .text_size(px(11.0)),
                    ),
            )
            .child(action_button(
                "browser-go",
                "GO",
                true,
                false,
                p,
                view.clone(),
                move |shell, window, cx| {
                    shell.navigate_browser(cx);
                    shell.sync_browser_input(window, cx);
                },
            ));

        let mut list = v_flex()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scrollbar()
            .gap_0();
        if entries.is_empty() {
            list = list.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .p_3()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.text_dim)
                            .child("No folders or program files here"),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(p.text_faint)
                            .child("Tap UP or type another folder path."),
                    ),
            );
        } else {
            for (index, entry) in entries.iter().enumerate() {
                list = list.child(browser_row(
                    entry,
                    index,
                    selected == Some(index),
                    p,
                    list_view.clone(),
                ));
            }
        }

        let mut recent_rows = v_flex().gap_1().p_1();
        if recent.is_empty() {
            recent_rows = recent_rows.child(
                div()
                    .text_size(px(9.5))
                    .text_color(p.text_faint)
                    .child("No recent files yet."),
            );
        }
        for (index, path) in recent.paths().iter().enumerate() {
            recent_rows = recent_rows.child(recent_row(path, index, p, recent_view.clone()));
        }

        let body = h_flex()
            .flex_1()
            .min_h(px(0.0))
            .items_stretch()
            .gap_0()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .h_full()
                    .child(list),
            )
            .child(
                v_flex()
                    .w(px(230.0))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .border_l_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .child(
                        h_flex()
                            .h(px(30.0))
                            .flex_shrink_0()
                            .items_center()
                            .px_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(section_label("RECENT", p)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scrollbar()
                            .child(recent_rows),
                    ),
            );

        let footer = h_flex()
            .h(px(TOUCH_TARGET + 12.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_t_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text_dim)
                    .child(selected_name),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(9.0))
                    .text_color(if error.is_some() {
                        p.danger
                    } else {
                        p.text_faint
                    })
                    .child(error.unwrap_or(path_text)),
            )
            .child(action_button(
                "browser-open",
                "OPEN",
                browser.selected_entry().is_some(),
                true,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.open_selected(cx);
                },
            ));

        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(920.0))
                    .max_w_full()
                    .h(px(660.0))
                    .max_h_full()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(header)
                    .child(path_row)
                    .child(body)
                    .child(footer),
            )
            .into_any_element()
    }

    fn error_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(message) = self.load_error.clone() else {
            return div().into_any_element();
        };
        let dismiss_view = view;
        h_flex()
            .min_h(px(36.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.danger)
            .bg(p.danger.alpha(0.12))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.danger)
                    .child("LOAD FAILED"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text)
                    .child(message),
            )
            .child(
                div()
                    .id("load-error-dismiss")
                    .h(px(28.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, _, cx| {
                        dismiss_view.update(cx, |shell, cx| {
                            shell.load_error = None;
                            cx.notify();
                        });
                    })
                    .child("DISMISS"),
            )
            .into_any_element()
    }
}

impl HasViewer for NcaShell {
    fn viewer_handle(&self) -> &ViewerHandle {
        &self.viewer
    }
}

impl Render for NcaShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.ensure_index();
        let viewport = window.viewport_size();
        let layout =
            ShellLayout::for_viewport(f32::from(viewport.width), f32::from(viewport.height));
        let p = Palette::for_mode(self.state.config.theme);
        if self.pending_search_reset {
            self.pending_search_reset = false;
            self.program_search_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let view = cx.entity();
        let font = cx.theme().font_family.clone();

        let mut root = v_flex()
            .size_full()
            .bg(p.bg)
            .text_color(p.text)
            .font_family(font)
            .on_drop::<ExternalPaths>({
                let drop_view = view.clone();
                move |paths, _, cx| {
                    let paths = paths.paths().to_vec();
                    drop_view.update(cx, |shell, cx| shell.drop_paths(&paths, cx));
                }
            })
            .child(self.top_bar(layout, p, view.clone()))
            .child(self.status_strip(p))
            .child(self.error_strip(p, view.clone()))
            .child(self.workspace(layout, p, view.clone()));

        if self.browser.is_some() {
            root = root.child(self.browser_sheet(layout, p, view.clone()));
        }
        if self.state.settings_open {
            root = root.child(self.settings_sheet(layout, p, view.clone()));
        }
        if self.legend_open {
            root = root.child(self.legend_sheet(layout, p, view));
        }

        root
    }
}

/// Height of the portrait program card after the viewer, legend, gaps, and
/// page padding are reserved. The viewer keeps at least half the viewport and
/// the program card fills the rest of the scroll column instead of leaving an
/// empty band at the bottom.
fn portrait_program_height(layout: ShellLayout) -> f32 {
    let reserved = layout.viewer_height + TOUCH_TARGET + 4.0 * crate::layout::GAP;
    (layout.content_height() - reserved).max(320.0)
}

/// A filled chip.
fn chip(text: &str, p: Palette) -> Div {
    div()
        .h(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .px_2()
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(p.text_dim)
        .child(text.to_string())
}

/// A section label in the parent's 11 px uppercase style.
fn section_label(text: &str, p: Palette) -> Div {
    div()
        .text_size(px(11.0))
        .font_weight(FontWeight::BOLD)
        .text_color(p.text_dim)
        .child(text.to_string())
}

/// A 48 px top-bar action.
fn action_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    active: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(TOUCH_TARGET))
        .min_w(px(52.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded_lg()
        .border_1()
        .border_color(if active { p.accent } else { p.border })
        .bg(if active { p.accent_muted } else { p.panel_alt })
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(if active {
            p.accent
        } else if enabled {
            p.text
        } else {
            p.text_faint
        });
    if enabled {
        button
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover).text_color(p.text))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| action(shell, window, cx));
            })
            .child(label)
    } else {
        button.opacity(0.45).child(label)
    }
}

/// A disabled placeholder action that keeps its top-bar slot.
fn placeholder_button(id: &'static str, label: &'static str, p: Palette) -> Stateful<Div> {
    let hint = format!("{label} arrives in a later milestone");
    div()
        .id(id)
        .h(px(TOUCH_TARGET))
        .min_w(px(52.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded_lg()
        .border_1()
        .border_color(p.border)
        .bg(p.panel_alt)
        .opacity(0.45)
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(p.text_faint)
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .child(label)
}

fn theme_button_id(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::System => "theme-system",
        ThemeMode::Dark => "theme-dark",
        ThemeMode::Light => "theme-light",
        ThemeMode::HighContrast => "theme-high-contrast",
    }
}

/// Full-window sheet layer below the top bar.
fn overlay_layer(layout: ShellLayout, p: Palette) -> Div {
    div()
        .absolute()
        .top(px(layout.overlay_top()))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .bg(p.scrim)
}

/// One 48 px program row.
fn program_row(
    line: &SourceLine,
    has_error: bool,
    selected: bool,
    p: Palette,
    mono: SharedString,
    view: Entity<NcaShell>,
) -> impl IntoElement {
    let number = line.number;
    let n_word = line.n_word;
    let text = line.text.clone();
    let trimmed = text.trim_start();
    let is_comment = trimmed.starts_with('(') || trimmed.starts_with(';');
    let text_color = if selected {
        p.accent
    } else if has_error {
        p.danger
    } else if is_comment {
        p.text_faint
    } else {
        p.text
    };
    let background = if selected { p.accent_muted } else { p.panel };
    let border = if selected {
        p.accent
    } else if has_error {
        p.danger
    } else {
        p.panel
    };
    h_flex()
        .id(format!("program-line-{number}"))
        .h(px(TOUCH_TARGET))
        .items_center()
        .gap_2()
        .px_2()
        .bg(background)
        .border_l(px(3.0))
        .border_color(border)
        .cursor_pointer()
        .hover(|style| style.bg(p.raised))
        .on_click(move |_, _, cx| {
            view.update(cx, |shell, cx| {
                shell.state.selected_line = Some(number);
                cx.notify();
            });
        })
        .child(
            div()
                .w(px(10.0))
                .h(px(10.0))
                .flex_shrink_0()
                .rounded(px(5.0))
                .border_1()
                .border_color(if has_error { p.danger } else { p.border_strong })
                .bg(if has_error { p.danger } else { p.panel }),
        )
        .child(
            div()
                .w(px(48.0))
                .flex_shrink_0()
                .text_size(px(9.5))
                .font_weight(if selected {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if selected { p.accent } else { p.text_faint })
                .child(number.to_string()),
        )
        .child(
            div()
                .w(px(38.0))
                .flex_shrink_0()
                .text_size(px(9.0))
                .text_color(p.text_faint)
                .child(n_word.map_or_else(String::new, |word| format!("N{word}"))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.5))
                .font_family(mono)
                .text_color(text_color)
                .child(text),
        )
}

/// One 48 px browser row.
fn browser_row(
    entry: &BrowserEntry,
    index: usize,
    selected: bool,
    p: Palette,
    view: Entity<NcaShell>,
) -> Stateful<Div> {
    let is_dir = entry.is_dir;
    let name = entry.name.clone();
    let kind = entry.kind_label();
    let meta = if is_dir {
        "FOLDER".to_string()
    } else {
        format_size(entry.size)
    };
    div()
        .id(format!("browser-entry-{index}"))
        .h(px(TOUCH_TARGET))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .bg(if selected { p.accent_muted } else { p.panel })
        .border_l(px(3.0))
        .border_color(if selected { p.accent } else { p.panel })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised))
        .on_click(move |_, window, cx| {
            view.update(cx, |shell, cx| shell.browser_click(index, window, cx));
        })
        .child(
            div()
                .w(px(42.0))
                .flex_shrink_0()
                .text_size(px(8.0))
                .font_weight(FontWeight::BOLD)
                .text_color(if is_dir { p.warning } else { p.text_faint })
                .child(kind),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.0))
                .text_color(if is_dir { p.info } else { p.text })
                .child(name),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(9.0))
                .text_color(p.text_faint)
                .child(meta),
        )
}

/// One 48 px recent-file row with its own remove action.
fn recent_row(path: &Path, index: usize, p: Palette, view: Entity<NcaShell>) -> Stateful<Div> {
    let open_path = path.to_path_buf();
    let remove_path = path.to_path_buf();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_path(path));
    let tail = compact_path(path, 34);
    let missing = !path.is_file();
    let open_view = view.clone();
    let remove_view = view;
    h_flex()
        .id(format!("recent-{index}"))
        .h(px(TOUCH_TARGET))
        .items_center()
        .gap_2()
        .px_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .cursor_pointer()
        .hover(|style| style.bg(p.raised_hover))
        .on_click(move |_, _, cx| {
            let path = open_path.clone();
            open_view.update(cx, |shell, cx| shell.open_path(path, cx));
        })
        .child(
            v_flex()
                .flex_1()
                .min_w(px(32.0))
                .gap_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if missing { p.text_faint } else { p.text })
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(7.5))
                        .text_color(if missing { p.danger } else { p.text_faint })
                        .child(if missing {
                            format!("MISSING | {tail}")
                        } else {
                            tail
                        }),
                ),
        )
        .child(
            div()
                .id(format!("recent-remove-{index}"))
                .w(px(32.0))
                .h(px(32.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_size(px(9.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .hover(|style| style.bg(p.panel_alt).text_color(p.danger))
                .on_click(move |_, _, cx| {
                    remove_view.update(cx, |shell, cx| {
                        shell.state.config.remove_recent(&remove_path);
                        cx.notify();
                    });
                })
                .child("X"),
        )
}

/// One legend swatch: colour plus the stroke pattern from [`MoveKind::style`].
fn legend_swatch(style: nc_viewer::MoveStyle, color: Hsla) -> Div {
    let mut swatch = h_flex().w(px(30.0)).h(px(16.0)).items_center().gap_1();
    match style.dash {
        None => {
            swatch = swatch.child(div().w(px(22.0)).h(px(2.0)).bg(color));
        }
        Some([on, off]) => {
            let width = (on * 0.5).clamp(2.0, 5.0);
            let count = if on > 3.0 { 3 } else { 4 };
            let gap = if off > 2.5 { 3.0 } else { 2.0 };
            for _ in 0..count {
                swatch = swatch
                    .child(div().w(px(width)).h(px(2.0)).bg(color))
                    .child(div().w(px(gap)));
            }
        }
    }
    if let Some(marker) = style.marker {
        let text = match marker {
            nc_viewer::HoleMarker::Ring => "O",
            nc_viewer::HoleMarker::DashedRing => "~O",
            nc_viewer::HoleMarker::DoubleRing => "OO",
        };
        swatch = swatch.child(
            div()
                .text_size(px(7.5))
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(text),
        );
    }
    swatch
}

/// Five-step cut-depth ramp shown under the kinds.
fn cut_depth_ramp(p: Palette) -> Div {
    let mut ramp = h_flex().gap_1().items_center();
    for index in 0..5 {
        #[allow(clippy::cast_precision_loss)]
        let fraction = index as f32 / 4.0;
        ramp = ramp.child(
            div()
                .flex_1()
                .h(px(10.0))
                .rounded(px(2.0))
                .bg(crate::theme::cut_depth_color(fraction, p)),
        );
    }
    v_flex()
        .gap_1()
        .p_2()
        .border_t_1()
        .border_color(p.border)
        .child(section_label("CUT DEPTH", p))
        .child(ramp)
        .child(
            div()
                .text_size(px(7.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child("SHALLOW -> DEEP"),
        )
}

/// All 13 kinds grouped by family, in legend order.
fn legend_groups() -> Vec<(MoveFamily, Vec<MoveKind>)> {
    MoveFamily::ALL
        .into_iter()
        .map(|family| {
            let kinds = MoveKind::ALL
                .into_iter()
                .filter(|kind| kind.family() == family)
                .collect::<Vec<_>>();
            (family, kinds)
        })
        .collect()
}

/// Representative colour for a collapsed family summary.
fn family_color(family: MoveFamily, p: Palette) -> Hsla {
    match family {
        MoveFamily::Travel => move_color(MoveKind::Rapid, p),
        MoveFamily::Cutting => move_color(MoveKind::Feed, p),
        MoveFamily::Arcs => move_color(MoveKind::ArcCw, p),
        MoveFamily::Holes => move_color(MoveKind::Drill, p),
        MoveFamily::Special => move_color(MoveKind::Probe, p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_groups_cover_every_kind_once() {
        let groups = legend_groups();
        assert_eq!(groups.len(), MoveFamily::ALL.len());
        let mut seen = 0;
        for (family, kinds) in groups {
            assert!(!kinds.is_empty(), "{family:?}");
            for kind in kinds {
                assert_eq!(kind.family(), family);
                seen += 1;
            }
        }
        assert_eq!(seen, MoveKind::ALL.len());
        assert_eq!(seen, 13);
    }

    #[test]
    fn every_legend_row_has_label_and_style() {
        for kind in MoveKind::ALL {
            let style = move_style(kind);
            assert!(!style.label.is_empty(), "{kind:?}");
            assert!(!kind.family().label().is_empty());
            assert!(!kind.short_label().is_empty());
        }
    }

    #[test]
    fn family_colors_are_deterministic() {
        let palette = Palette::dark();
        for family in MoveFamily::ALL {
            assert_eq!(family_color(family, palette), family_color(family, palette));
        }
    }
}
