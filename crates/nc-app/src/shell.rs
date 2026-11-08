//! The compact read-only desktop shell.
//!
//! One workspace, two reserved modes, and three sheets: file open, move
//! legend, and settings. The viewer dominates the window. There is no
//! controller panel, DRO, jog, MDI, console, connect, reset, or start control;
//! the only file mutation is reading a local program into memory.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, ExternalPaths, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, ScrollStrategy,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use nc_core::{Tool, ToolShape, ToolTable};
use nc_gcode::Program;
use nc_sim::{SimConfig, Simulation};
use nc_stock::{Cutter, StockModel};
use nc_viewer::{MoveFamily, MoveKind, StockBox};

use crate::file_browser::{
    BrowserEntry, FileBrowser, compact_path, default_root, display_path, format_size, read_program,
};
use crate::layout::{STATUS_BAR_HEIGHT, ShellLayout, TOUCH_TARGET};
use crate::program::SourceLine;
use crate::sim::{
    FrameClock, HeightfieldCache, HeightfieldMesh, PlaybackSpeed, SegmentLines, SimCanvas,
    SimToggles, ToolChip, Transport, follow_visible_row, format_diameter, format_time,
    fraction_for_time, is_long_seek, parse_tool_edit, time_for_fraction, tool_chip,
};
use crate::sim_worker::SeekWorker;
use crate::state::{AppMode, LoadedProgram, NcaConfig, NcaState, ParseState};
use crate::theme::{Palette, ThemeMode, apply as apply_theme, move_color, move_style};
use crate::viewer::{self, HasViewer, ViewerHandle, ViewerMode};

/// Height of the SIMULATE transport bar, in logical pixels.
const TRANSPORT_HEIGHT: f32 = 76.0;

/// One active simulation and the data the shell needs to paint it.
struct SimSession {
    simulation: Simulation,
    program: Arc<Program>,
    stock: StockModel,
    tools: ToolTable,
    segment_lines: Arc<SegmentLines>,
}

impl SimSession {
    fn active_tool(&self) -> u32 {
        let snapshot = self.simulation.snapshot();
        if snapshot.tool > 0 {
            snapshot.tool
        } else {
            self.program.stats.tools.first().copied().unwrap_or(0)
        }
    }

    fn active_cutter(&self) -> Option<Cutter> {
        self.simulation
            .cutter_for_tool(self.active_tool())
            .cloned()
            .or_else(|| self.simulation.current_cutter().cloned())
    }

    fn canvas(&self, mesh: Arc<HeightfieldMesh>, toggles: SimToggles) -> Option<SimCanvas> {
        let stock = self.stock.to_stock_box()?;
        let snapshot = self.simulation.snapshot();
        let cutter = self.active_cutter();
        let tool_assumed = cutter
            .as_ref()
            .is_some_and(|cutter| cutter.assumed || cutter.is_centerline());
        Some(SimCanvas {
            snapshot,
            mesh,
            stock,
            cutter,
            segment_lines: Arc::clone(&self.segment_lines),
            toggles,
            tool_assumed,
        })
    }
}

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
    sim: Option<SimSession>,
    transport: Transport,
    frame_clock: FrameClock,
    toggles: SimToggles,
    mesh_cache: HeightfieldCache,
    mesh_updated_at: Option<Instant>,
    seek_worker: Option<SeekWorker>,
    seeking: bool,
    seek_generation: Option<u64>,
    scrub_target: Option<f64>,
    sim_error: Option<String>,
    last_follow_line: Option<usize>,
    view_selected_line: Option<usize>,
    tool_popover_open: bool,
    tool_edit_number: Option<u32>,
    tool_shape_draft: ToolShape,
    tool_diameter_input: Entity<InputState>,
    tool_flutes_input: Entity<InputState>,
    tool_edit_error: Option<String>,
    scrub_slider: Entity<SliderState>,
}

impl NcaShell {
    /// Builds the shell and loads the built-in demo.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        window: &mut Window,
        open_path: Option<PathBuf>,
        view_3d: bool,
        open_browser: bool,
        simulate: bool,
        sim_at: Option<f32>,
        autoplay: bool,
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
        let tool_diameter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("6.0")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let tool_flutes_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("2")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let scrub_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.001)
                .default_value(0.0)
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
        subscriptions.push(cx.subscribe(&scrub_slider, |this, _, event, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => *value,
            };
            let SliderValue::Single(fraction) = value else {
                return;
            };
            this.scrub_to_fraction(f64::from(fraction), cx);
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
            sim: None,
            transport: Transport::new(),
            frame_clock: FrameClock::default(),
            toggles: SimToggles::default(),
            mesh_cache: HeightfieldCache::default(),
            mesh_updated_at: None,
            seek_worker: None,
            seeking: false,
            seek_generation: None,
            scrub_target: None,
            sim_error: None,
            last_follow_line: None,
            view_selected_line: None,
            tool_popover_open: false,
            tool_edit_number: None,
            tool_shape_draft: ToolShape::Flat,
            tool_diameter_input,
            tool_flutes_input,
            tool_edit_error: None,
            scrub_slider,
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
        if simulate {
            shell.enter_simulate(cx);
            if let Some(fraction) = sim_at {
                shell.startup_seek(window, f64::from(fraction), cx);
            }
            if autoplay {
                shell.start_playback(cx);
            }
        }
        shell
    }

    /// Builds a deterministic simulation for the loaded program.
    fn build_sim_session(&mut self) -> Result<(), String> {
        let loaded = self
            .state
            .loaded
            .as_ref()
            .ok_or_else(|| "No program is open.".to_string())?;
        let program = loaded.program.as_ref().cloned().ok_or_else(|| {
            "The file has parse errors, so there is no geometry to simulate.".to_string()
        })?;
        let stock = loaded
            .stock
            .ok_or_else(|| "No stock box could be inferred for this file.".to_string())?;
        let tools = ToolTable::resolve(&self.state.config.tools, &program.tool_hints);
        let simulation = Simulation::new(&program, stock, &tools, &SimConfig::default())
            .map_err(|error| format!("Cannot simulate this program: {error}"))?;
        let segment_lines = Arc::new(SegmentLines::from_program(&program));
        self.sim = Some(SimSession {
            simulation,
            program,
            stock,
            tools,
            segment_lines,
        });
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        Ok(())
    }

    fn enter_simulate(&mut self, cx: &mut Context<Self>) {
        if self.state.mode == AppMode::Simulate {
            return;
        }
        let previous_selection = self.state.selected_line;
        match self.build_sim_session() {
            Ok(()) => {
                self.view_selected_line = previous_selection;
                self.state.mode = AppMode::Simulate;
                self.sim_error = None;
                self.transport.stop();
                self.frame_clock.reset();
                self.toggles.stock = self.viewer.stock_visible();
                self.refresh_sim_mesh(true);
                self.follow_current_line(true, cx);
            }
            Err(message) => {
                self.sim = None;
                self.mesh_cache.clear();
                self.sim_error = Some(message);
                self.state.mode = AppMode::View;
            }
        }
        cx.notify();
    }

    fn leave_simulate(&mut self, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            return;
        }
        self.cancel_seek();
        self.transport.pause();
        self.frame_clock.reset();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.state.mode = AppMode::View;
        self.state.selected_line = self.view_selected_line.take();
        cx.notify();
    }

    /// Drops playback and rebuilds if SIMULATE is still the active mode.
    fn program_changed(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.view_selected_line = None;
        self.frame_clock.reset();
        self.transport.stop();
        self.sim_error = None;
        if self.state.mode == AppMode::Simulate {
            match self.build_sim_session() {
                Ok(()) => {
                    self.toggles.stock = self.viewer.stock_visible();
                    self.refresh_sim_mesh(true);
                    self.follow_current_line(true, cx);
                }
                Err(message) => {
                    self.sim = None;
                    self.mesh_cache.clear();
                    self.sim_error = Some(message);
                    self.state.mode = AppMode::View;
                }
            }
        }
        cx.notify();
    }

    fn cancel_seek(&mut self) {
        if let Some(worker) = self.seek_worker.as_mut() {
            worker.cancel();
        }
        self.seeking = false;
        self.seek_generation = None;
        self.scrub_target = None;
    }

    /// Applies a startup seek synchronously so a screenshot is reproducible.
    fn startup_seek(&mut self, window: &mut Window, fraction: f64, cx: &mut Context<Self>) {
        let Some(actual) = self.sim.as_mut().map(|session| {
            let total = session.simulation.timeline().total_time_s();
            let target = time_for_fraction(fraction, total);
            session.simulation.seek_to_time(target);
            session.simulation.snapshot().progress
        }) else {
            return;
        };
        self.transport.pause();
        self.frame_clock.reset();
        self.cancel_seek();
        self.mesh_cache.clear();
        self.refresh_sim_mesh(true);
        self.last_follow_line = None;
        self.follow_current_line(true, cx);
        self.set_scrub_slider(actual, window, cx);
        cx.notify();
    }

    fn refresh_sim_mesh(&mut self, force: bool) {
        let now = Instant::now();
        let playing = self.transport.is_playing();
        if !force
            && playing
            && let Some(last) = self.mesh_updated_at
            && now.duration_since(last) < Duration::from_millis(33)
        {
            return;
        }
        let Some(session) = self.sim.as_ref() else {
            return;
        };
        let base = session.simulation.config().render_resolution;
        let resolution = if playing {
            base
        } else {
            (base.saturating_mul(2)).clamp(8, 512)
        };
        let _ = self
            .mesh_cache
            .ensure(session.simulation.heightmap(), &session.stock, resolution);
        self.mesh_updated_at = Some(now);
    }

    fn sim_canvas(&self) -> Option<SimCanvas> {
        let session = self.sim.as_ref()?;
        let mesh = self.mesh_cache.current()?;
        session.canvas(mesh, self.toggles)
    }

    fn follow_current_line(&mut self, scroll: bool, cx: &mut Context<Self>) {
        let Some(line) = self
            .sim
            .as_ref()
            .map(|session| session.simulation.snapshot().current_line)
        else {
            return;
        };
        if line == 0 {
            return;
        }
        let line_changed = self.last_follow_line != Some(line);
        if line_changed {
            self.last_follow_line = Some(line);
        }
        if self.state.selected_line != Some(line) {
            self.state.selected_line = Some(line);
        }
        if scroll && line_changed {
            self.state.ensure_index();
            if let Some(index) = self.state.index().cloned()
                && let Some(row) = follow_visible_row(&index, line)
            {
                self.program_scroll
                    .scroll_to_item(row, ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    fn set_scrub_slider(&mut self, fraction: f64, window: &mut Window, cx: &mut Context<Self>) {
        #[allow(clippy::cast_possible_truncation)]
        let value = fraction.clamp(0.0, 1.0) as f32;
        self.scrub_slider.update(cx, |state, cx| {
            state.set_value(SliderValue::Single(value), window, cx);
        });
    }

    fn sync_scrub_slider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sim.as_ref() else {
            return;
        };
        #[allow(clippy::cast_possible_truncation)]
        let fraction =
            self.scrub_target
                .unwrap_or_else(|| session.simulation.snapshot().progress) as f32;
        let current = match self.scrub_slider.read(cx).value() {
            SliderValue::Single(value) => value,
            SliderValue::Range(_, end) => end,
        };
        if (current - fraction).abs() > 0.0005 {
            self.set_scrub_slider(f64::from(fraction), window, cx);
        }
    }

    fn tick_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            self.frame_clock.reset();
            return;
        }
        if self.seeking {
            window.request_animation_frame();
            return;
        }
        if !self.transport.is_playing() {
            self.frame_clock.reset();
            return;
        }
        let now = Instant::now();
        let dt = self.frame_clock.step(now);
        if dt > 0.0 {
            let factor = self.transport.speed().factor();
            let mut finished = false;
            if let Some(session) = self.sim.as_mut() {
                session.simulation.advance(dt * factor);
                let snapshot = session.simulation.snapshot();
                let total = session.simulation.timeline().total_time_s();
                finished = total > 0.0 && snapshot.time_s + 1.0e-9 >= total;
            }
            if finished {
                self.transport.pause();
                self.frame_clock.reset();
            }
            self.follow_current_line(true, cx);
            self.refresh_sim_mesh(false);
        }
        window.request_animation_frame();
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        if self.state.mode == AppMode::Simulate && self.sim.is_some() {
            self.transport.play();
            self.frame_clock.reset();
            cx.notify();
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate || self.sim.is_none() || self.seeking {
            return;
        }
        if self.transport.is_playing() {
            self.transport.pause();
            self.frame_clock.reset();
        } else {
            let at_end = self
                .sim
                .as_ref()
                .is_some_and(|session| session.simulation.snapshot().progress >= 1.0);
            if at_end && let Some(session) = self.sim.as_mut() {
                session.simulation.reset();
                self.mesh_cache.clear();
                self.mesh_updated_at = None;
            }
            self.transport.play();
            self.frame_clock.reset();
            self.follow_current_line(true, cx);
        }
        cx.notify();
    }

    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.stop();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.reset();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn step_motion(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.step();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.step_motion();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn step_line(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.step();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.step_line();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn scrub_to_fraction(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let Some(total) = self
            .sim
            .as_ref()
            .map(|session| session.simulation.timeline().total_time_s())
        else {
            return;
        };
        let target = time_for_fraction(fraction, total);
        self.scrub_target = Some(fraction.clamp(0.0, 1.0));
        self.request_seek_to_time(target, cx);
    }

    fn seek_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            self.state.selected_line = Some(line);
            cx.notify();
            return;
        }
        let Some((target, total)) = self.sim.as_ref().map(|session| {
            let total = session.simulation.timeline().total_time_s();
            let target = session
                .simulation
                .timeline()
                .time_for_line_at_or_after(line)
                .unwrap_or(total);
            (target, total)
        }) else {
            self.state.selected_line = Some(line);
            cx.notify();
            return;
        };
        self.scrub_target = Some(fraction_for_time(target, total));
        self.request_seek_to_time(target, cx);
    }

    fn request_seek_to_time(&mut self, target_s: f64, cx: &mut Context<Self>) {
        let Some((total, current, program, stock, tools, config)) =
            self.sim.as_ref().map(|session| {
                (
                    session.simulation.timeline().total_time_s(),
                    session.simulation.snapshot().time_s,
                    Arc::clone(&session.program),
                    session.stock,
                    session.tools.clone(),
                    session.simulation.config().clone(),
                )
            })
        else {
            return;
        };
        let target = target_s.clamp(0.0, total);
        self.transport.pause();
        self.frame_clock.reset();
        if !is_long_seek(current, target) {
            if let Some(session) = self.sim.as_mut() {
                session.simulation.seek_to_time(target);
            }
            self.seeking = false;
            self.seek_generation = None;
            self.scrub_target = None;
            self.mesh_cache.clear();
            self.mesh_updated_at = None;
            self.follow_current_line(true, cx);
            self.refresh_sim_mesh(true);
            cx.notify();
            return;
        }

        if self.seek_worker.is_none() {
            match SeekWorker::spawn() {
                Ok(worker) => self.seek_worker = Some(worker),
                Err(message) => {
                    self.sim_error = Some(message);
                    self.seeking = false;
                    self.scrub_target = None;
                    cx.notify();
                    return;
                }
            }
        }
        self.seeking = true;
        if let Some(worker) = self.seek_worker.as_mut() {
            match worker.request(program, stock, tools, config, target) {
                Ok(generation) => self.seek_generation = Some(generation),
                Err(message) => {
                    self.sim_error = Some(message);
                    self.seeking = false;
                    self.seek_generation = None;
                    self.scrub_target = None;
                }
            }
        }
        cx.notify();
    }

    fn poll_seek(&mut self, cx: &mut Context<Self>) {
        let Some(worker) = self.seek_worker.as_ref() else {
            return;
        };
        let mut response = None;
        while let Some(candidate) = worker.try_recv() {
            if Some(candidate.generation) == self.seek_generation {
                response = Some(candidate);
                break;
            }
        }
        let Some(response) = response else {
            if self.seeking {
                let dead = self
                    .seek_worker
                    .as_ref()
                    .is_some_and(|worker| !worker.is_alive());
                if dead {
                    self.seeking = false;
                    self.seek_generation = None;
                    self.scrub_target = None;
                    self.sim_error = Some("The seek worker stopped unexpectedly.".to_string());
                    cx.notify();
                }
            }
            return;
        };
        self.seeking = false;
        self.seek_generation = None;
        self.scrub_target = None;
        match response.result {
            Ok(simulation) => {
                if let Some(session) = self.sim.as_mut() {
                    session.simulation = simulation;
                }
                self.transport.pause();
                self.frame_clock.reset();
                self.mesh_cache.clear();
                self.mesh_updated_at = None;
                self.follow_current_line(true, cx);
                self.refresh_sim_mesh(true);
            }
            Err(message) => {
                self.sim_error = Some(format!("Seek failed: {message}"));
            }
        }
        cx.notify();
    }

    fn resolved_tools(&self) -> ToolTable {
        self.state
            .loaded
            .as_ref()
            .and_then(|loaded| {
                loaded.program.as_ref().map(|program| {
                    ToolTable::resolve(&self.state.config.tools, &program.tool_hints)
                })
            })
            .unwrap_or_default()
    }

    fn active_tool_number(&self) -> u32 {
        if let Some(session) = self.sim.as_ref() {
            return session.active_tool();
        }
        self.state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.program.as_ref())
            .and_then(|program| {
                program
                    .stats
                    .tools
                    .iter()
                    .copied()
                    .find(|tool| *tool > 0)
                    .or_else(|| program.stats.tools.first().copied())
            })
            .unwrap_or(0)
    }

    fn current_tool_chip(&self) -> ToolChip {
        if let Some(session) = self.sim.as_ref() {
            let tool = session.active_tool();
            let cutter = session.active_cutter();
            return tool_chip(tool, session.tools.get(tool), cutter.as_ref());
        }
        let Some(program) = self
            .state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.program.as_ref())
        else {
            return ToolChip {
                tool: 0,
                text: "NO TOOL".to_string(),
                assumed: false,
            };
        };
        let tool = program
            .stats
            .tools
            .iter()
            .copied()
            .find(|tool| *tool > 0)
            .or_else(|| program.stats.tools.first().copied())
            .unwrap_or(0);
        if tool == 0 {
            return ToolChip {
                tool: 0,
                text: "NO TOOL".to_string(),
                assumed: false,
            };
        }
        let tools = self.resolved_tools();
        let cutter = tools
            .get(tool)
            .map(|entry| Cutter::from_tool(entry, None))
            .unwrap_or_else(Cutter::centerline);
        tool_chip(tool, tools.get(tool), Some(&cutter))
    }

    fn open_tool_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tool = self.active_tool_number();
        if tool == 0 {
            self.tool_edit_number = None;
            self.tool_edit_error = Some("No tool is named by this program.".to_string());
            self.tool_popover_open = true;
            cx.notify();
            return;
        }
        let tools = if let Some(session) = self.sim.as_ref() {
            session.tools.clone()
        } else {
            self.resolved_tools()
        };
        let entry = tools.get(tool).cloned().unwrap_or_else(|| Tool::new(tool));
        self.tool_shape_draft = entry.shape;
        let diameter = entry.diameter_mm.map(format_diameter).unwrap_or_default();
        let flutes = entry
            .flutes
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.tool_diameter_input.update(cx, |input, cx| {
            input.set_value(diameter, window, cx);
        });
        self.tool_flutes_input.update(cx, |input, cx| {
            input.set_value(flutes, window, cx);
        });
        self.tool_edit_number = Some(tool);
        self.tool_edit_error = None;
        self.tool_popover_open = true;
        cx.notify();
    }

    fn apply_tool_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(number) = self.tool_edit_number else {
            self.tool_popover_open = false;
            cx.notify();
            return;
        };
        let diameter_text = self.tool_diameter_input.read(cx).value().to_string();
        let flutes_text = self.tool_flutes_input.read(cx).value().to_string();
        let existing = self.state.config.tools.get(number);
        let tool = match parse_tool_edit(
            number,
            &diameter_text,
            &flutes_text,
            self.tool_shape_draft,
            existing,
        ) {
            Ok(tool) => tool,
            Err(message) => {
                self.tool_edit_error = Some(message);
                cx.notify();
                return;
            }
        };
        self.state.config.set_tool(tool);
        self.tool_popover_open = false;
        self.tool_edit_error = None;
        if self.state.mode == AppMode::Simulate {
            self.rebuild_after_tool_edit(window, cx);
        } else {
            cx.notify();
        }
    }

    fn rebuild_after_tool_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.transport.stop();
        self.frame_clock.reset();
        match self.build_sim_session() {
            Ok(()) => {
                self.sim_error = None;
                self.toggles.stock = self.viewer.stock_visible();
                self.refresh_sim_mesh(true);
                self.follow_current_line(true, cx);
            }
            Err(message) => {
                self.sim = None;
                self.mesh_cache.clear();
                self.sim_error = Some(message);
                self.state.mode = AppMode::View;
            }
        }
        cx.notify();
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
                        shell.program_changed(cx);
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

    /// The clickable tool chip shared by the top bar and the SIMULATE HUD.
    fn tool_chip_button(&self, id: &'static str, p: Palette, view: Entity<Self>) -> AnyElement {
        let info = self.current_tool_chip();
        div()
            .id(id)
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_lg()
            .border_1()
            .border_color(if info.assumed { p.warning } else { p.border })
            .bg(p.panel_alt)
            .text_size(px(9.5))
            .font_weight(FontWeight::BOLD)
            .text_color(if info.assumed { p.warning } else { p.text })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| shell.open_tool_popover(window, cx));
            })
            .child(info.text)
            .into_any_element()
    }

    fn top_bar(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let target = layout.touch_target();
        let view_chip = action_button(
            "mode-view",
            "VIEW",
            true,
            self.state.mode == AppMode::View,
            p,
            view.clone(),
            move |shell, _, cx| shell.leave_simulate(cx),
        );
        let simulate_chip = action_button(
            "mode-simulate",
            "SIMULATE",
            true,
            self.state.mode == AppMode::Simulate,
            p,
            view.clone(),
            move |shell, _, cx| shell.enter_simulate(cx),
        );

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
                    shell.toggles.stock = next;
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
        let tool_button = self.tool_chip_button("bar-tool", p, view.clone());

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
            .child(tool_button)
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
                        .child(view_chip)
                        .child(simulate_chip),
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
                    .child(view_chip)
                    .child(simulate_chip),
            )
            .child(actions)
            .into_any_element()
    }

    fn tool_chip_compact(&self, id: &'static str, p: Palette, view: Entity<Self>) -> AnyElement {
        let info = self.current_tool_chip();
        div()
            .id(id)
            .h(px(26.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(if info.assumed { p.warning } else { p.border })
            .bg(p.panel_alt)
            .text_size(px(9.0))
            .font_weight(FontWeight::BOLD)
            .text_color(if info.assumed { p.warning } else { p.text })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| shell.open_tool_popover(window, cx));
            })
            .child(info.text)
            .into_any_element()
    }

    fn transport_bar(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let playing = self.transport.is_playing();
        let state = if self.seeking {
            "SEEKING"
        } else {
            self.transport.state().label()
        };
        let state_color = if self.seeking {
            p.info
        } else if playing {
            p.accent
        } else {
            p.text_dim
        };

        let play = transport_button(
            "sim-play",
            if playing { "PAUSE" } else { "PLAY" },
            true,
            playing,
            p,
            view.clone(),
            move |shell, _, cx| shell.toggle_playback(cx),
        );
        let stop = transport_button(
            "sim-stop",
            "STOP",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.stop_playback(cx),
        );
        let step_motion = transport_button(
            "sim-step-motion",
            "STEP MOVE",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.step_motion(cx),
        );
        let step_line = transport_button(
            "sim-step-line",
            "STEP LINE",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.step_line(cx),
        );

        let mut speed_row = h_flex().items_center().gap_1();
        for speed in PlaybackSpeed::ALL {
            speed_row = speed_row.child(transport_button(
                speed_button_id(speed),
                speed.label(),
                true,
                self.transport.speed() == speed,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.transport.set_speed(speed);
                    cx.notify();
                },
            ));
        }

        let (time_text, tick_text, line_text) = self.sim.as_ref().map_or_else(
            || {
                (
                    "0:00.0 / 0:00.0".to_string(),
                    "TICK 0".to_string(),
                    "L-".to_string(),
                )
            },
            |session| {
                let snapshot = session.simulation.snapshot();
                let total = session.simulation.timeline().total_time_s();
                let kind = snapshot
                    .current_kind
                    .map_or("--", nc_viewer::MoveKind::label);
                (
                    format!("{} / {}", format_time(snapshot.time_s), format_time(total)),
                    format!("TICK {}", snapshot.tick),
                    format!("L{} {}", snapshot.current_line, kind),
                )
            },
        );

        let row_one = h_flex()
            .h(px(30.0))
            .items_center()
            .gap_1()
            .child(
                chip(state, p)
                    .text_color(state_color)
                    .border_color(if self.seeking { p.info } else { p.border }),
            )
            .child(play)
            .child(stop)
            .child(step_motion)
            .child(step_line)
            .child(div().w(px(4.0)))
            .child(speed_row)
            .child(div().flex_1())
            .child(chip(&time_text, p))
            .child(chip(&tick_text, p))
            .child(chip(&line_text, p));

        let slider = div()
            .flex_1()
            .min_w(px(140.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .child(
                Slider::new(&self.scrub_slider)
                    .horizontal()
                    .w_full()
                    .h(px(20.0)),
            );

        let mut toggles = h_flex().items_center().gap_1();
        toggles = toggles
            .child(compact_toggle(
                "sim-toggle-stock",
                "STOCK",
                self.toggles.stock,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.stock = !shell.toggles.stock;
                    shell.viewer.set_stock_visible(shell.toggles.stock);
                    shell.state.stock_visible = shell.toggles.stock;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-rapids",
                "RAPIDS",
                self.toggles.rapids,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.rapids = !shell.toggles.rapids;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-air",
                "AIR",
                self.toggles.air_cuts,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.air_cuts = !shell.toggles.air_cuts;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-future",
                "FUTURE",
                self.toggles.future_path,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.future_path = !shell.toggles.future_path;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-tool",
                "TOOL",
                self.toggles.tool,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.tool = !shell.toggles.tool;
                    cx.notify();
                },
            ))
            .child(
                compact_toggle(
                    "sim-toggle-warnings",
                    "WARN",
                    self.toggles.warnings,
                    p,
                    view.clone(),
                    move |shell, _, cx| {
                        shell.toggles.warnings = !shell.toggles.warnings;
                        cx.notify();
                    },
                )
                .tooltip(|window, cx| {
                    Tooltip::new(
                        "No diagnostics are loaded in this build; the warning overlay is reserved.",
                    )
                    .build(window, cx)
                }),
            );

        let row_two = h_flex()
            .h(px(28.0))
            .items_center()
            .gap_2()
            .child(slider)
            .child(toggles)
            .child(self.tool_chip_compact("sim-tool-chip", p, view));

        v_flex()
            .h(px(TRANSPORT_HEIGHT))
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel_alt)
            .child(row_one)
            .child(row_two)
            .into_any_element()
    }

    fn sim_hud(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(session) = self.sim.as_ref() else {
            return div().into_any_element();
        };
        let snapshot = session.simulation.snapshot();
        let stats = session.simulation.stats();
        let total = session.simulation.timeline().total_time_s();
        let tool = self.current_tool_chip();
        let mut rows = v_flex().gap(px(2.0));
        let row = |label: &'static str, value: String, color: Hsla| {
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(76.0))
                        .flex_shrink_0()
                        .text_size(px(8.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_faint)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(value),
                )
        };
        rows = rows
            .child(row("TICK", snapshot.tick.to_string(), p.text))
            .child(row(
                "TIME",
                format!("{} / {}", format_time(snapshot.time_s), format_time(total)),
                p.text,
            ))
            .child(row(
                "LINE",
                format!(
                    "L{} {}",
                    snapshot.current_line,
                    snapshot
                        .current_kind
                        .map_or("--", nc_viewer::MoveKind::label)
                ),
                p.text,
            ))
            .child(row(
                "PROGRESS",
                format!("{:.1}%", snapshot.progress * 100.0),
                p.text,
            ))
            .child(row(
                "CUT VOLUME",
                if stats.volume_assumed {
                    format!("{:.1} mm3 (assumed)", stats.cut_volume_mm3)
                } else {
                    format!("{:.1} mm3", stats.cut_volume_mm3)
                },
                if stats.volume_assumed {
                    p.warning
                } else {
                    p.text
                },
            ))
            .child(row(
                "MAX DEPTH",
                format!("{:.2} mm", stats.max_depth_mm),
                p.text,
            ))
            .child(row(
                "CUT / RAPID",
                format!(
                    "{} / {}",
                    format_time(stats.cut_time_s),
                    format_time(stats.rapid_time_s)
                ),
                p.text_dim,
            ))
            .child(row("AIR TIME", format_time(stats.air_time_s), p.text_dim));

        let mut badges = h_flex().gap_1().items_center();
        if tool.assumed {
            badges = badges.child(
                chip("TOOL SIZE ASSUMED", p)
                    .text_color(p.warning)
                    .border_color(p.warning),
            );
        }
        if session.simulation.is_coarse() {
            badges = badges.child(
                chip("COARSE GRID", p)
                    .text_color(p.warning)
                    .border_color(p.warning),
            );
        }
        if self.seeking {
            badges = badges.child(chip("SEEKING", p).text_color(p.info).border_color(p.info));
        }

        v_flex()
            .absolute()
            .top(px(40.0))
            .left(px(8.0))
            .w(px(258.0))
            .gap_1()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border_strong)
            .bg(p.panel.alpha(0.94))
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_dim)
                            .child("SIMULATION"),
                    )
                    .child(div().flex_1())
                    .child(self.tool_chip_compact("hud-tool-chip", p, view)),
            )
            .child(badges)
            .child(rows)
            .into_any_element()
    }

    fn sim_error_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(message) = self.sim_error.clone() else {
            return div().into_any_element();
        };
        h_flex()
            .min_h(px(36.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.warning)
            .bg(p.warning.alpha(0.10))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.warning)
                    .child("SIMULATION UNAVAILABLE"),
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
                    .id("sim-error-dismiss")
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
                        view.update(cx, |shell, cx| {
                            shell.sim_error = None;
                            cx.notify();
                        });
                    })
                    .child("DISMISS"),
            )
            .into_any_element()
    }

    fn tool_popover(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        if !self.tool_popover_open {
            return div().into_any_element();
        }
        let number = self.tool_edit_number;
        let title = number.map_or_else(|| "TOOL".to_string(), |number| format!("TOOL T{number}"));
        let mut shapes = h_flex().gap_1().flex_wrap();
        for shape in [
            ToolShape::Flat,
            ToolShape::Ball,
            ToolShape::Bull,
            ToolShape::Drill,
            ToolShape::VBit,
            ToolShape::Unknown,
        ] {
            shapes = shapes.child(compact_toggle(
                tool_shape_id(shape),
                shape.label().to_ascii_uppercase().as_str(),
                self.tool_shape_draft == shape,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.tool_shape_draft = shape;
                    cx.notify();
                },
            ));
        }
        let mut body = v_flex().gap_2();
        if let Some(number) = number {
            body = body
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("DIAMETER MM"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(34.0))
                                .rounded_lg()
                                .border_1()
                                .border_color(p.border_strong)
                                .bg(p.panel_alt)
                                .px_1()
                                .child(
                                    Input::new(&self.tool_diameter_input)
                                        .h(px(32.0))
                                        .w_full()
                                        .text_size(px(12.0)),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("FLUTES"),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .h(px(34.0))
                                .rounded_lg()
                                .border_1()
                                .border_color(p.border_strong)
                                .bg(p.panel_alt)
                                .px_1()
                                .child(
                                    Input::new(&self.tool_flutes_input)
                                        .h(px(32.0))
                                        .w_full()
                                        .text_size(px(12.0)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(8.5))
                                .text_color(p.text_faint)
                                .child("Empty = unknown"),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("SHAPE"),
                        )
                        .child(shapes),
                );
            let apply_view = view.clone();
            body = body.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(8.5))
                            .text_color(p.text_faint)
                            .child(format!(
                                "T{number}: local tool data, never written to the G-code."
                            )),
                    )
                    .child(action_button(
                        "tool-apply",
                        "APPLY",
                        true,
                        true,
                        p,
                        apply_view,
                        move |shell, window, cx| shell.apply_tool_edit(window, cx),
                    )),
            );
        }
        if let Some(error) = &self.tool_edit_error {
            body = body.child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.warning)
                    .child(error.clone()),
            );
        }
        let top = layout.top_bar_height
            + STATUS_BAR_HEIGHT
            + if self.state.mode == AppMode::Simulate {
                TRANSPORT_HEIGHT
            } else {
                0.0
            }
            + 8.0;
        div()
            .absolute()
            .top(px(top))
            .right(px(12.0))
            .w(px(330.0))
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border_strong)
            .bg(p.panel)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .child(section_label(&title, p))
                    .child(chip("QUICK EDIT", p))
                    .child(div().flex_1())
                    .child(action_button(
                        "tool-popover-close",
                        "CLOSE",
                        true,
                        false,
                        p,
                        view,
                        move |shell, _, cx| {
                            shell.tool_popover_open = false;
                            cx.notify();
                        },
                    )),
            )
            .child(body)
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
        let sim = self.sim_canvas();
        let marker = if sim.is_none() {
            viewer::program_start_marker(program.as_deref())
        } else {
            None
        };
        let visibility = self.state.visibility();
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
            sim.clone(),
            view.clone(),
        ));

        let mut top_left = h_flex()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .gap_1()
            .child(chip(mode.label(), p))
            .child(chip(&format!("{zoom}%"), p));
        if self.state.mode == AppMode::Simulate
            && let Some(sim) = &sim
        {
            top_left = top_left
                .child(
                    chip(self.transport.state().label(), p)
                        .text_color(if self.transport.is_playing() {
                            p.accent
                        } else {
                            p.text_dim
                        })
                        .border_color(if self.transport.is_playing() {
                            p.accent
                        } else {
                            p.border
                        }),
                )
                .child(chip(&format!("L{}", sim.snapshot.current_line), p));
        }
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

        if self.state.mode == AppMode::Simulate {
            card = card.child(self.sim_hud(p, view.clone()));
            if let Some(sim) = &sim
                && sim.tool_assumed
            {
                let pose = sim.snapshot.pose_mm;
                if let Some((x, y)) = self.viewer.project_pose(pose, mode) {
                    card = card.child(
                        chip("\u{00d8}?", p)
                            .absolute()
                            .left(px(x + 10.0))
                            .top(px((y - 22.0).max(4.0)))
                            .text_color(p.warning)
                            .border_color(p.warning),
                    );
                }
            }
        }

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
        let mut column = v_flex().gap_1().p_2().child(cut_depth_ramp(p));
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
        column
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
        if self.state.mode == AppMode::Simulate {
            self.poll_seek(cx);
            self.tick_playback(window, cx);
            self.refresh_sim_mesh(false);
            self.sync_scrub_slider(window, cx);
        }
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
            .child(self.sim_error_strip(p, view.clone()))
            .child(self.error_strip(p, view.clone()));
        if self.state.mode == AppMode::Simulate {
            root = root.child(self.transport_bar(p, view.clone()));
        }
        root = root.child(self.workspace(layout, p, view.clone()));

        if self.tool_popover_open {
            root = root.child(self.tool_popover(layout, p, view.clone()));
        }
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

/// A 34 px transport action.
fn transport_button(
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
        .h(px(30.0))
        .min_w(px(44.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(if active { p.accent } else { p.border })
        .bg(if active { p.accent_muted } else { p.panel })
        .text_size(px(9.0))
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

/// A compact SIMULATE toggle chip.
fn compact_toggle(
    id: &'static str,
    label: &str,
    on: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(if on { p.accent } else { p.border })
        .bg(if on { p.accent_muted } else { p.panel })
        .text_size(px(8.0))
        .font_weight(FontWeight::BOLD)
        .text_color(if on { p.accent } else { p.text_faint })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised_hover))
        .on_click(move |_, window, cx| {
            view.update(cx, |shell, cx| action(shell, window, cx));
        })
        .child(label.to_string())
}

fn speed_button_id(speed: PlaybackSpeed) -> &'static str {
    match speed {
        PlaybackSpeed::Half => "sim-speed-half",
        PlaybackSpeed::One => "sim-speed-one",
        PlaybackSpeed::Two => "sim-speed-two",
        PlaybackSpeed::Four => "sim-speed-four",
    }
}

fn tool_shape_id(shape: ToolShape) -> &'static str {
    match shape {
        ToolShape::Flat => "tool-shape-flat",
        ToolShape::Ball => "tool-shape-ball",
        ToolShape::Bull => "tool-shape-bull",
        ToolShape::Drill => "tool-shape-drill",
        ToolShape::VBit => "tool-shape-vbit",
        ToolShape::Unknown => "tool-shape-unknown",
    }
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
                shell.seek_to_line(number, cx);
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
