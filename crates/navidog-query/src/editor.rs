//! Query editor tab (30-query-editor.md §2): toolbar, SQL editor and the results area.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{
    Copy, Cut, Editor, EditorState, Enter, Escape, IndentInline, InputEvent, MoveDown, MoveUp,
    OutdentInline, TabSize,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, actions,
    div, px,
};
use navidog_core::actions::OpenConnectionId;
use navidog_core::object::{ObjectRef, Scope};
use navidog_core::settings::{ResultsPosition, Settings, ShowAfterRun};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent};
use navidog_ui::assets::lucide;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tabs, tokens};
use navidog_util::sql_format::CommentMode;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::results::{CloseAllResults, CloseResultTab, Results, TogglePinTab, UnpinAllTabs};
use crate::run::{Live, RunKind};

pub(crate) const PANEL: &str = "QueryEditor";
pub(crate) const CONTEXT: &str = "QueryEditor";

actions!(
    query,
    [
        Run,
        RunCurrent,
        RunSelected,
        Stop,
        Explain,
        ToggleContinueOnError,
        ToggleKeepResults,
        ToggleWordWrap,
        ZoomIn,
        ZoomOut,
        ActualSize,
        ResultsBelow,
        ResultsBeside,
        ResultsNewPage,
        ShowResults,
        ShowResultAfterRun,
        ShowMessagesAfterRun,
        ShowSummaryAfterRun,
        ToggleProfile,
        UpdateOutline,
    ]
);

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct PickConnection {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct PickDatabase {
    pub name: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct PickSchema {
    pub name: SharedString,
}

/// Inputs inside a query editor: binding here overrides the text field's own keys (⌘. is "code
/// actions" and ⌘F the built-in search there).
const IN_EDITOR: &str = "QueryEditor > Input";

pub(crate) fn init(cx: &mut App) {
    use crate::find::FindPrevious;
    use crate::format::{PasteFromStack, ToggleComment};
    use navidog_grid::{FindNext, ShowFind, ShowReplace};
    cx.bind_keys([
        KeyBinding::new("cmd-r", Run, Some(CONTEXT)),
        KeyBinding::new("shift-cmd-r", RunCurrent, Some(CONTEXT)),
        KeyBinding::new("cmd-.", Stop, Some(CONTEXT)),
        KeyBinding::new("cmd-.", Stop, Some(IN_EDITOR)),
        KeyBinding::new("cmd-f", ShowFind, Some(CONTEXT)),
        KeyBinding::new("cmd-f", ShowFind, Some(IN_EDITOR)),
        KeyBinding::new("alt-cmd-f", ShowReplace, Some(IN_EDITOR)),
        KeyBinding::new("alt-cmd-f", ShowReplace, Some(CONTEXT)),
        KeyBinding::new("cmd-g", FindNext, Some(IN_EDITOR)),
        KeyBinding::new("shift-cmd-g", FindPrevious, Some(IN_EDITOR)),
        KeyBinding::new("cmd-/", ToggleComment, Some(IN_EDITOR)),
        KeyBinding::new("shift-cmd-v", PasteFromStack, Some(IN_EDITOR)),
        KeyBinding::new("cmd-s", crate::save::SaveQuery, Some(CONTEXT)),
        KeyBinding::new("shift-cmd-s", crate::save::SaveQueryAs, Some(CONTEXT)),
        KeyBinding::new("cmd-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("cmd-0", ActualSize, Some(CONTEXT)),
    ]);
    register_panel(cx, PANEL, |ctx, window, cx| {
        let info = match ctx.info() {
            PanelInfo::Panel(v) => v.clone(),
            _ => serde_json::Value::Null,
        };
        panel_handle(QueryEditor::restore(info, window, cx))
    });
}

/// What the editor's text belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuerySource {
    /// Not saved yet; the number tells "Untitled" tabs apart
    Untitled(usize),
    /// A saved query (`ObjectKind::Query`)
    Saved(ObjectRef),
    /// Any `.sql` file opened from disk
    External(PathBuf),
}

/// Where the editor runs, completes and saves (the toolbar pickers).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub connection: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl Target {
    pub fn scope(&self) -> Option<Scope> {
        Some(Scope::new(
            self.connection.clone()?,
            self.database.clone()?,
            self.schema.clone(),
        ))
    }
}

/// Panel state of untitled and external editors. Saved queries dump their `ObjectRef` instead,
/// so favorites, recent objects and the startup screen work on them like on tables.
#[derive(Serialize, Deserialize)]
struct Dumped {
    query: DumpedQuery,
}

#[derive(Serialize, Deserialize)]
struct DumpedQuery {
    id: String,
    source: QuerySource,
    target: Target,
}

static UNTITLED: AtomicUsize = AtomicUsize::new(0);

pub fn next_untitled() -> usize {
    UNTITLED.fetch_add(1, Ordering::Relaxed) + 1
}

pub struct QueryEditor {
    focus_handle: FocusHandle,
    /// Names the autosave file (30 §8)
    pub(crate) id: String,
    pub(crate) source: QuerySource,
    pub(crate) target: Target,
    pub(crate) editor: Entity<EditorState>,
    /// Changed since it was opened or saved
    pub(crate) dirty: bool,
    /// Modification time of the file when it was loaded or saved, to notice outside edits
    pub(crate) file_mtime: Option<std::time::SystemTime>,
    /// PostgreSQL schemas of the picked database
    pub(crate) schemas: Vec<String>,
    pub(crate) live: Option<Live>,
    pub(crate) running: bool,
    /// Set by Stop: the statements after the current one are skipped
    pub(crate) stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Run again once the picked connection has opened
    pub(crate) pending_run: Option<RunKind>,
    /// Last values typed for each `[$name]` parameter, and whether they were raw
    pub(crate) params: HashMap<String, (String, bool)>,
    pub(crate) results: Results,
    /// "Show results in a new page": the results page is showing instead of the editor
    pub(crate) results_page: bool,
    /// Editor preferences last applied (line numbers, folding, tab width, spaces)
    applied: Option<(bool, bool, u8, bool)>,
    /// Highlighting is SQL, so completion may run. Cached: the completion trigger
    /// runs inside the editor update and must not read that editor.
    pub(crate) sql_highlight: bool,
    /// Snippet placeholders being tabbed through
    pub(crate) tab_stops: Option<crate::snippet::TabStops>,
    /// Snippet codes the completion menu offered last
    pub(crate) offered_snippets: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    /// An arrow key moved the completion menu since it opened (for "auto select first item")
    pub(crate) menu_moved: bool,
    pub(crate) find: crate::find::Find,
    pub(crate) stack_paste: Option<crate::format::StackPaste>,
    /// Bracket highlight frames and the caret they were computed for
    pub(crate) brackets: Option<gpui_kit::component::input::RangeDecorationCollection>,
    pub(crate) bracket_caret: Option<usize>,
    /// Zoomed font size; `None` follows the preferences
    pub(crate) zoom: Option<f32>,
    pub(crate) word_wrap: bool,
    /// The user answered the close prompt
    close_approved: bool,
    pub(crate) task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for QueryEditor {}

impl Focusable for QueryEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for QueryEditor {
    fn panel_name(&self) -> &'static str {
        PANEL
    }

    fn dump(&self, _: &App) -> PanelState {
        let info = match &self.source {
            QuerySource::Saved(o) => serde_json::to_value(o),
            source => serde_json::to_value(Dumped {
                query: DumpedQuery {
                    id: self.id.clone(),
                    source: source.clone(),
                    target: self.target.clone(),
                },
            }),
        };
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(info.unwrap_or_default()),
        }
    }

    /// Unsaved text makes the tab ask first; see `tabs::CloseGuard`.
    fn closable(&self, _: &App) -> bool {
        self.close_approved || !self.dirty
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = PanelId::from(cx.entity_id());
        tabs::added(panel, group, cx);
        let (editor, home) = (cx.entity().downgrade(), window.window_handle());
        let guard: tabs::CloseGuard = std::rc::Rc::new(move |cx| {
            let editor = editor.clone();
            let _ = home.update(cx, |_, window, cx| {
                let _ = editor.update(cx, |e, cx| e.ask_to_close(window, cx));
            });
        });
        tabs::set_close_guard(panel, guard, cx);
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = PanelId::from(cx.entity_id());
        tabs::set_active(panel, active, cx);
        tabs::set_surface(
            panel,
            active,
            tabs::MenuSurface::Query,
            tabs::TableMenu::default(),
            cx,
        );
        if active {
            set_active_editor(cx.entity().downgrade(), cx);
            self.publish_context(cx);
            if let QuerySource::Saved(o) = &self.source {
                let o = o.clone();
                Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Object(o), cx));
            }
            self.check_external_change(window, cx);
        } else {
            Workspace::global(cx).update(cx, |w, cx| w.set_query_context(None, cx));
        }
    }

    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if tabs::removed(PanelId::from(cx.entity_id()), cx) {
            return;
        }
        Workspace::global(cx).update(cx, |w, cx| w.set_query_context(None, cx));
        if let Some(live) = self.live.take() {
            live.close();
        }
    }
}

impl Panel for QueryEditor {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = self
            .target
            .connection
            .as_ref()
            .and_then(|id| ConnectionStore::global(cx).read(cx).get(id))
            .and_then(|c| c.color.clone());
        let conn = self.connection_name(cx);
        let label = tabs::object_label(
            &self.display_name(),
            self.target.database.as_deref(),
            (!conn.is_empty()).then_some(conn.as_str()),
        );
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("file-code")),
            label,
            color.and_then(|c| tokens::connection_color(&c, cx)),
            self.dirty && !self.close_approved,
        )
    }
}

/// The query editor tab last activated, for commands from outside it (inserting identifiers).
struct ActiveEditor(WeakEntity<QueryEditor>);
impl gpui_kit::Global for ActiveEditor {}

fn set_active_editor(editor: WeakEntity<QueryEditor>, cx: &mut App) {
    cx.set_global(ActiveEditor(editor));
}

pub fn active_editor(cx: &App) -> Option<Entity<QueryEditor>> {
    cx.try_global::<ActiveEditor>()?.0.upgrade()
}

impl QueryEditor {
    pub fn new(
        source: QuerySource,
        target: Target,
        text: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::with_id(uuid_v4(), source, target, text, window, cx)
    }

    fn with_id(
        id: String,
        source: QuerySource,
        target: Target,
        text: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let s = Settings::global(cx).clone();
        let sql_highlight = language_for(&s, text.len()) == "sql";
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language_for(&s, text.len()))
                .line_number(s.show_line_numbers)
                .folding(s.code_folding)
                .soft_wrap(s.word_wrap)
                .auto_close(false)
                .searchable(false)
                .tab_size(TabSize {
                    tab_size: s.tab_width.clamp(1, 16) as usize,
                    hard_tabs: !s.insert_spaces,
                })
        });
        editor.update(cx, |e, cx| e.set_value(text, window, cx));
        let this = cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.subscribe_in(&editor, window, |this, _, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::Change) {
                        this.on_text_changed(cx);
                        this.expand_inserted_snippet(window, cx);
                    }
                }),
                cx.subscribe_in(
                    &Sessions::global(cx),
                    window,
                    |this, _, e: &SessionsEvent, window, cx| this.on_sessions_event(e, window, cx),
                ),
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
                // Caret moves only notify the editor; bracket frames follow them from here
                cx.observe(&editor, |this, _, cx| this.sync_brackets(cx)),
                // Editor preferences apply to open editors right away (30 §9)
                cx.observe_global_in::<Settings>(window, |this, window, cx| {
                    this.apply_settings(window, cx)
                }),
            ];
            let results = Results::new(window, cx);
            let find = crate::find::Find::new(window, cx);
            let offered: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
            let provider = crate::completion::Provider {
                editor: cx.entity().downgrade(),
                offered: offered.clone(),
            };
            editor.update(cx, |e, _| {
                e.lsp_mut().completion_provider = Some(std::rc::Rc::new(provider));
                e.lsp_mut().set_completion_highlighter(std::rc::Rc::new(
                    crate::completion::highlight_label,
                ));
                // raw-px: room for "column · type" labels
                e.lsp_mut().completion_menu.max_width = px(420.);
            });
            Self {
                focus_handle: cx.focus_handle(),
                id,
                source,
                target,
                file_mtime: None,
                editor,
                dirty: false,
                schemas: Vec::new(),
                live: None,
                running: false,
                stop_flag: Default::default(),
                pending_run: None,
                params: HashMap::new(),
                results,
                results_page: false,
                applied: Some((
                    s.show_line_numbers,
                    s.code_folding,
                    s.tab_width,
                    s.insert_spaces,
                )),
                sql_highlight,
                tab_stops: None,
                offered_snippets: offered,
                menu_moved: false,
                find,
                stack_paste: None,
                brackets: None,
                bracket_caret: None,
                zoom: None,
                word_wrap: s.word_wrap,
                close_approved: false,
                task: None,
                _subscriptions: subs,
            }
        });
        this.update(cx, |this, cx| {
            this.file_mtime = this
                .file_path()
                .and_then(|p| navidog_core::queries::modified(&p));
            this.load_schemas(cx);
            this.ensure_outline(false, cx);
        });
        crate::save::register(this.downgrade(), cx);
        this
    }

    /// An editor brought back from an autosave: its text is unsaved.
    pub(crate) fn restored(
        id: String,
        source: QuerySource,
        target: Target,
        text: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let editor = Self::with_id(id, source, target, text, window, cx);
        editor.update(cx, |e, _| e.dirty = true);
        editor
    }

    /// File > Open External File > Query…: any `.sql` file, saved back in place.
    pub fn open_external(
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let scope = selection.scope();
        let target = Target {
            connection: selection.connection_id().cloned(),
            database: scope.as_ref().map(|s| s.database.clone()),
            schema: scope.and_then(|s| s.schema),
        };
        Self::new(QuerySource::External(path), target, text, window, cx)
    }

    /// Runs everything (the navigator's "Open Query").
    pub fn run_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(RunKind::All, window, cx);
    }

    /// "Query Table": an untitled editor holding `sql`, then the shell runs it.
    pub fn from_sql(
        target: Target,
        sql: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::new(
            QuerySource::Untitled(next_untitled()),
            target,
            sql,
            window,
            cx,
        )
    }

    /// A new untitled query at the workspace selection (⌘Y, toolbar "New Query").
    pub fn untitled(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let scope = selection.scope();
        let target = Target {
            connection: selection.connection_id().cloned(),
            database: scope.as_ref().map(|s| s.database.clone()),
            schema: scope.and_then(|s| s.schema),
        };
        Self::new(
            QuerySource::Untitled(next_untitled()),
            target,
            String::new(),
            window,
            cx,
        )
    }

    /// Opens a saved query (double-click in the navigator, favorites, recent objects).
    pub fn open_saved(object: ObjectRef, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let text = navidog_core::queries::read(&object.scope, &object.name).unwrap_or_default();
        let target = Target {
            connection: Some(object.scope.connection.clone()),
            database: Some(object.scope.database.clone()),
            schema: object.scope.schema.clone(),
        };
        Self::new(QuerySource::Saved(object), target, text, window, cx)
    }

    /// Recreates a tab from the dock layout.
    fn restore(info: serde_json::Value, window: &mut Window, cx: &mut App) -> Entity<Self> {
        if let Ok(o) = serde_json::from_value::<ObjectRef>(info.clone()) {
            return Self::open_saved(o, window, cx);
        }
        match serde_json::from_value::<Dumped>(info) {
            Ok(Dumped { query }) => {
                let text = match &query.source {
                    QuerySource::External(path) => {
                        std::fs::read_to_string(path).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                if let QuerySource::Untitled(n) = query.source {
                    UNTITLED.fetch_max(n, Ordering::Relaxed);
                }
                Self::with_id(query.id, query.source, query.target, text, window, cx)
            }
            Err(_) => Self::new(
                QuerySource::Untitled(next_untitled()),
                Target::default(),
                String::new(),
                window,
                cx,
            ),
        }
    }

    pub fn display_name(&self) -> String {
        match &self.source {
            QuerySource::Untitled(1) => t!("query.untitled").to_string(),
            QuerySource::Untitled(n) => format!("{} {n}", t!("query.untitled")),
            QuerySource::Saved(o) => o.name.clone(),
            QuerySource::External(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    pub fn source(&self) -> &QuerySource {
        &self.source
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Inserts at the caret (info pane identifiers and snippets). A snippet's `${…}` placeholders
    /// become tab stops.
    pub fn insert_text(
        &mut self,
        text: &str,
        snippet: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if snippet {
            self.insert_snippet(text, window, cx);
            return;
        }
        let text = text.to_string();
        self.editor.update(cx, |e, cx| {
            e.insert(text, window, cx);
            e.focus(window, cx);
        });
    }

    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    /// Replaces the text (one undo step) and marks the query modified.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.editor.read(cx).text().len();
        let text = text.to_string();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(0..len, cx);
            e.replace(text, window, cx);
        });
    }

    /// Explains everything (the toolbar's "Explain").
    pub fn explain_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(RunKind::Explain, window, cx);
    }

    pub(crate) fn dialect(&self, cx: &App) -> Option<Dialect> {
        let id = self.target.connection.as_deref()?;
        Sessions::global(cx).read(cx).dialect(id).or_else(|| {
            let driver = ConnectionStore::global(cx).read(cx).get(id)?.driver.clone();
            Some(match driver.as_str() {
                "postgres" => Dialect::Postgres,
                "sqlite" => Dialect::Sqlite,
                _ => Dialect::MySql,
            })
        })
    }

    /// Line numbers, folding, tabs and highlighting follow the preferences.
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Settings::global(cx).clone();
        let lang = language_for(&s, self.editor.read(cx).text().len());
        self.sql_highlight = lang == "sql";
        let wanted = (
            s.show_line_numbers,
            s.code_folding,
            s.tab_width,
            s.insert_spaces,
        );
        if self.applied == Some(wanted) && self.editor.read(cx).language_name() == lang {
            return;
        }
        self.applied = Some(wanted);
        self.editor.update(cx, |e, cx| {
            e.set_line_number(s.show_line_numbers, window, cx);
            e.set_folding(s.code_folding, window, cx);
            e.set_tab_size(
                TabSize {
                    tab_size: s.tab_width.clamp(1, 16) as usize,
                    hard_tabs: !s.insert_spaces,
                },
                cx,
            );
            if e.language_name() != lang {
                e.set_highlighter(lang, cx);
            }
        });
        if !s.bracket_highlight
            && let Some(d) = &self.brackets
        {
            d.clear(cx);
            self.bracket_caret = None;
        }
        cx.notify();
    }

    fn on_text_changed(&mut self, cx: &mut Context<Self>) {
        self.menu_moved = false;
        // Past the size limit the editor turns plain text and completion stops (F-PREF-004)
        let s = Settings::global(cx);
        let lang = language_for(s, self.editor.read(cx).text().len());
        self.sql_highlight = lang == "sql";
        if self.editor.read(cx).language_name() != lang {
            self.editor.update(cx, |e, cx| e.set_highlighter(lang, cx));
        }
        if !self.dirty {
            self.dirty = true;
            cx.notify();
        }
        // Errors marked by the last run go once the text changes
        self.editor.update(cx, |e, cx| {
            if let Some(d) = e.diagnostics_mut()
                && !d.is_empty()
            {
                d.clear();
                cx.notify();
            }
        });
    }

    fn on_sessions_event(
        &mut self,
        e: &SessionsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = self.target.connection.clone() else {
            return;
        };
        match e {
            SessionsEvent::Opened(id) if *id == conn => {
                self.load_schemas(cx);
                self.ensure_outline(false, cx);
                if let Some(kind) = self.pending_run.take() {
                    self.run(kind, window, cx);
                }
                cx.notify();
            }
            SessionsEvent::Failed(id, _) if *id == conn => {
                self.pending_run = None;
                cx.notify();
            }
            // A query is a document: closing the connection only drops this editor's session,
            // the next run opens the connection again (30 §2)
            SessionsEvent::Closed(id) | SessionsEvent::Reopening(id) if *id == conn => {
                if let Some(live) = self.live.take() {
                    live.close();
                }
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn pick_connection(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.target.connection.as_deref() == Some(id.as_str()) {
            return;
        }
        let default_db = ConnectionStore::global(cx)
            .read(cx)
            .get(&id)
            .and_then(|c| c.general.database.clone())
            .filter(|d| !d.is_empty());
        self.target = Target {
            connection: Some(id.clone()),
            database: default_db,
            schema: None,
        };
        self.schemas.clear();
        if let Some(live) = self.live.take() {
            live.close();
        }
        if !Sessions::global(cx).read(cx).state(&id).is_open() {
            window.dispatch_action(Box::new(OpenConnectionId { id: id.into() }), cx);
        } else {
            self.load_schemas(cx);
        }
        self.target_changed(cx);
        cx.notify();
    }

    pub(crate) fn pick_database(&mut self, name: String, cx: &mut Context<Self>) {
        if self.target.database.as_deref() == Some(name.as_str()) {
            return;
        }
        self.target.database = Some(name);
        self.target.schema = None;
        self.schemas.clear();
        self.load_schemas(cx);
        self.ensure_outline(false, cx);
        self.target_changed(cx);
        cx.notify();
    }

    pub(crate) fn pick_schema(&mut self, name: String, cx: &mut Context<Self>) {
        self.target.schema = Some(name);
        self.ensure_outline(false, cx);
        self.target_changed(cx);
        cx.notify();
    }

    /// PostgreSQL: lists the schemas of the picked database; picks `public` when none is set.
    fn load_schemas(&mut self, cx: &mut Context<Self>) {
        let (Some(conn), Some(db)) = (self.target.connection.clone(), self.target.database.clone())
        else {
            return;
        };
        if self.dialect(cx) != Some(Dialect::Postgres) {
            return;
        }
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .run_rel(&conn, Some(&db), |r| r.list_schemas())
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(Ok(list)) = job.await else { return };
            let _ = this.update(cx, |this, cx| {
                let user: Vec<String> = list
                    .into_iter()
                    .filter(|s| !Dialect::Postgres.is_system_schema(s))
                    .collect();
                if this.target.schema.is_none() && user.iter().any(|s| s == "public") {
                    this.target.schema = Some("public".into());
                    this.ensure_outline(false, cx);
                    this.target_changed(cx);
                }
                this.schemas = user;
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_to_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty {
            self.close_now(window, cx);
            return;
        }
        let this = cx.entity();
        let name = self.display_name();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (save, discard, enter) = (this.clone(), this.clone(), this.clone());
            alert
                .icon(lucide("info"))
                .title(t!("query.close_unsaved", name = name).to_string())
                .on_ok(move |_, window, cx| {
                    enter.update(cx, |e, cx| e.save_and_close(window, cx));
                    true
                })
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("query-close-cancel")
                                .label(t!("query.cancel").to_string())
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("query-close-discard")
                                .label(t!("query.dont_save").to_string())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    discard.update(cx, |e, cx| {
                                        crate::recovery::forget(&e.id);
                                        e.close_now(window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("query-close-save")
                                .primary()
                                .label(t!("query.save").to_string())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    save.update(cx, |e, cx| e.save_and_close(window, cx));
                                }),
                        ),
                )
        });
    }

    /// Saving an untitled query asks for a name first; the tab stays until it is saved.
    fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_query(true, window, cx);
    }

    pub(crate) fn close_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_approved = true;
        let close = tabs::CloseTabs {
            panel: Some(PanelId::from(cx.entity_id())),
            which: tabs::Close::This,
        };
        window.dispatch_action(Box::new(close), cx);
    }

    /// The editor font size: zoom, else the preferences, else the token.
    fn font_pt(&self, cx: &App) -> f32 {
        let s = Settings::global(cx);
        self.zoom.unwrap_or(if s.use_default_editor_font {
            tokens::query::EDITOR_FONT_SIZE.value()
        } else {
            s.editor_font_pt
        })
    }

    fn zoom_by(&mut self, step: f32, cx: &mut Context<Self>) {
        let next = (self.font_pt(cx) + step).clamp(
            tokens::query::EDITOR_FONT_MIN,
            tokens::query::EDITOR_FONT_MAX,
        );
        self.zoom = Some(next);
        cx.notify();
    }

    fn toggle_word_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.word_wrap = !self.word_wrap;
        let wrap = self.word_wrap;
        self.editor
            .update(cx, |e, cx| e.set_soft_wrap(wrap, window, cx));
        cx.notify();
    }

    /// Tells the info pane where this (active) editor points.
    pub(crate) fn publish_context(&self, cx: &mut Context<Self>) {
        let context = navidog_core::workspace::QueryContext {
            connection: self.target.connection.clone(),
            database: self.target.database.clone(),
            schema: self.target.schema.clone(),
        };
        Workspace::global(cx).update(cx, |w, cx| w.set_query_context(Some(context), cx));
    }

    fn target_changed(&mut self, cx: &mut Context<Self>) {
        let active = active_editor(cx).is_some_and(|e| e.entity_id() == cx.entity_id());
        if active {
            self.publish_context(cx);
        }
    }

    pub(crate) fn completion_open(&self, cx: &App) -> bool {
        self.editor.read(cx).completion_menu_state().open
    }

    /// With "auto select first item" off, Enter and Tab accept only after an arrow key.
    fn accepts_completion(&self, cx: &App) -> bool {
        Settings::global(cx).auto_select_first || self.menu_moved
    }

    pub(crate) fn editor_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Menu items dispatch along the focused element; keep focus inside this editor.
    pub(crate) fn focus_for_menu(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn connection_menu(&self, cx: &App) -> NativeMenu {
        let store = ConnectionStore::global(cx);
        store
            .read(cx)
            .connections()
            .iter()
            .fold(NativeMenu::new(), |menu, c| {
                menu.menu_with_check(
                    c.name.clone(),
                    self.target.connection.as_deref() == Some(c.id.as_str()),
                    Box::new(PickConnection {
                        id: c.id.clone().into(),
                    }),
                )
            })
    }

    fn database_menu(&self, cx: &App) -> NativeMenu {
        let Some(conn) = self.target.connection.as_deref() else {
            return NativeMenu::new();
        };
        let ConnState::Open { databases, .. } = Sessions::global(cx).read(cx).state(conn) else {
            return NativeMenu::new();
        };
        databases.iter().fold(NativeMenu::new(), |menu, db| {
            menu.menu_with_check(
                db.clone(),
                self.target.database.as_deref() == Some(db.as_str()),
                Box::new(PickDatabase {
                    name: db.clone().into(),
                }),
            )
        })
    }

    fn schema_menu(&self) -> NativeMenu {
        self.schemas.iter().fold(NativeMenu::new(), |menu, s| {
            menu.menu_with_check(
                s.clone(),
                self.target.schema.as_deref() == Some(s.as_str()),
                Box::new(PickSchema {
                    name: s.clone().into(),
                }),
            )
        })
    }

    fn run_menu(&self, cx: &App) -> NativeMenu {
        NativeMenu::new()
            .menu(t!("query.run").to_string(), Box::new(Run))
            .menu(t!("query.run_current").to_string(), Box::new(RunCurrent))
            .menu(t!("query.run_selected").to_string(), Box::new(RunSelected))
            .separator()
            .menu_with_check(
                t!("query.continue_on_error").to_string(),
                Settings::global(cx).continue_on_error,
                Box::new(ToggleContinueOnError),
            )
            .menu_with_check(
                t!("query.keep_results").to_string(),
                Settings::global(cx).keep_results,
                Box::new(ToggleKeepResults),
            )
    }

    fn has_selection(&self, cx: &App) -> bool {
        !self.editor.read(cx).selected_range().is_empty()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let button = |id: &'static str, icon: &'static str, label: String| {
            let mut glyph = lucide(icon);
            if let Some(color) = tokens::icon_color(icon, cx) {
                glyph = glyph.text_color(color);
            }
            Button::new(id)
                .small()
                .custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(p.label)
                        .hover(p.fill)
                        .active(p.chrome_active),
                )
                .icon(glyph)
                .label(label)
        };
        let picker = |id: &'static str, icon: &'static str, label: String, empty: bool| {
            let mut glyph = lucide(icon);
            if let Some(color) = tokens::icon_color(icon, cx) {
                glyph = glyph.text_color(color);
            }
            h_flex()
                .id(id)
                .flex_none()
                .w(tokens::query::PICKER_WIDTH.rems())
                .h(tokens::info::SEARCH_FIELD_HEIGHT.rems())
                .px_1()
                .gap_1()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(p.separator)
                .bg(p.fill_tertiary)
                .cursor_pointer()
                .child(glyph.size(tokens::navigator::ROW_ICON.rems()))
                .child(
                    truncated(SharedString::from(format!("{id}-label")), label)
                        .flex_1()
                        .text_color(if empty { p.label_secondary } else { p.label }),
                )
                .child(
                    lucide("chevron-down")
                        .size(tokens::navigator::ROW_ICON.rems())
                        .text_color(p.label_secondary),
                )
        };
        let store = ConnectionStore::global(cx);
        let conn_name = self
            .target
            .connection
            .as_ref()
            .and_then(|id| store.read(cx).get(id))
            .map(|c| c.name.clone());
        let pg = self.dialect(cx) == Some(Dialect::Postgres);
        let selected = self.has_selection(cx);
        let show_results = Settings::global(cx).results_position == ResultsPosition::NewPage
            && !self.results_page
            && self.results.has_run();
        v_flex()
            .child(
                h_flex()
                    .h(tokens::query::TOOLBAR_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .min_w_0()
                    .items_center()
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        button("query-save", "save", t!("query.save").to_string()).on_click(
                            cx.listener(|this, _, window, cx| this.save_query(false, window, cx)),
                        ),
                    )
                    .child(separator(cx))
                    .child(
                        button(
                            "query-beautify",
                            "wand-sparkles",
                            t!("query.beautify").to_string(),
                        )
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::format::Beautify), cx)
                        }),
                    )
                    .child(
                        button(
                            "query-snippets",
                            "scroll-text",
                            t!("query.snippets").to_string(),
                        )
                        .on_click(|_, window, cx| {
                            window
                                .dispatch_action(Box::new(navidog_core::actions::ShowSnippets), cx)
                        }),
                    )
                    .child(
                        button("query-builder", "blocks", t!("query.builder").to_string())
                            .disabled(true),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("query-focus-mode")
                            .ghost()
                            .small()
                            .icon(lucide("scan"))
                            .tooltip(t!("query.focus_mode").to_string())
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(navidog_core::actions::ToggleFocusMode),
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h(tokens::query::TOOLBAR_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .min_w_0()
                    .items_center()
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        picker(
                            "query-connection",
                            "plug",
                            conn_name
                                .clone()
                                .unwrap_or_else(|| t!("query.connection").to_string()),
                            conn_name.is_none(),
                        )
                        .on_click(cx.listener(
                            |this, e: &ClickEvent, window, cx| {
                                this.focus_for_menu(window, cx);
                                this.connection_menu(cx).show(e.position(), window, cx);
                            },
                        )),
                    )
                    .child(
                        picker(
                            "query-database",
                            "database",
                            self.target
                                .database
                                .clone()
                                .unwrap_or_else(|| t!("query.database").to_string()),
                            self.target.database.is_none(),
                        )
                        .when(self.target.connection.is_none(), |d| d.opacity(0.4))
                        .on_click(cx.listener(
                            |this, e: &ClickEvent, window, cx| {
                                if this.target.connection.is_none() {
                                    return;
                                }
                                this.focus_for_menu(window, cx);
                                this.database_menu(cx).show(e.position(), window, cx);
                            },
                        )),
                    )
                    .when(pg, |bar| {
                        bar.child(
                            picker(
                                "query-schema",
                                "layers",
                                self.target
                                    .schema
                                    .clone()
                                    .unwrap_or_else(|| t!("query.schema").to_string()),
                                self.target.schema.is_none(),
                            )
                            .when(self.schemas.is_empty(), |d| d.opacity(0.4))
                            .on_click(cx.listener(
                                |this, e: &ClickEvent, window, cx| {
                                    if this.schemas.is_empty() {
                                        return;
                                    }
                                    this.focus_for_menu(window, cx);
                                    this.schema_menu().show(e.position(), window, cx);
                                },
                            )),
                        )
                    })
                    .child(
                        button("query-run", "play", t!("query.run").to_string())
                            .disabled(self.running)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run(RunKind::All, window, cx)
                            })),
                    )
                    .child(
                        Button::new("query-run-menu")
                            .ghost()
                            .xsmall()
                            .icon(lucide("chevron-down"))
                            .disabled(self.running)
                            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                                this.focus_for_menu(window, cx);
                                this.run_menu(cx).show(e.position(), window, cx);
                            })),
                    )
                    .child(
                        Checkbox::new("query-continue")
                            .label(t!("query.continue_on_error").to_string())
                            .checked(Settings::global(cx).continue_on_error)
                            .on_click(|checked: &bool, _, cx| {
                                let on = *checked;
                                Settings::update(cx, |s| s.continue_on_error = on);
                            }),
                    )
                    .child(
                        button("query-stop", "circle-stop", t!("query.stop").to_string())
                            .disabled(!self.running)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                    .child(
                        button(
                            "query-explain",
                            "list-tree",
                            if selected {
                                t!("query.explain_selected").to_string()
                            } else {
                                t!("query.explain").to_string()
                            },
                        )
                        .disabled(self.running)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.run(RunKind::Explain, window, cx)
                            }),
                        ),
                    )
                    .when(show_results, |bar| {
                        bar.child(
                            button(
                                "query-show-results",
                                "square",
                                t!("query.show_results").to_string(),
                            )
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ShowResults), cx)
                            }),
                        )
                    }),
            )
    }

    /// The theme's mono font (`font.mono_family`); zoom and the preferences change only its size.
    fn render_editor(&self, cx: &App) -> impl IntoElement {
        let state = self.editor.clone();
        div().id("query-editor-text").size_full().min_h_0().child(
            Editor::new(&self.editor)
                .bordered(false)
                .size_full()
                // raw-px: font size in points set by zoom or the preferences
                .text_size(px(self.font_pt(cx)))
                .context_menu(move |menu, _, cx| context_menu(menu, &state, cx)),
        )
    }
}

/// Right-click menu of the SQL editor (30 §3). Items about the selection are disabled without one.
fn context_menu(menu: NativeMenu, state: &Entity<EditorState>, cx: &App) -> NativeMenu {
    use crate::format::{
        Beautify, CommentLines, LowerCase, PasteFromStack, UncommentLines, UpperCase,
    };
    use gpui_kit::component::input::{Paste, SelectAll};
    let selected = state.read(cx).selected_text().to_string();
    let none = selected.is_empty();
    menu.menu_with_disabled(t!("query.cut").to_string(), none, Box::new(Cut))
        .menu_with_disabled(t!("query.copy").to_string(), none, Box::new(Copy))
        .menu(t!("query.paste").to_string(), Box::new(Paste))
        .menu(
            t!("query.paste_from_stack").to_string(),
            Box::new(PasteFromStack),
        )
        .menu(t!("query.select_all").to_string(), Box::new(SelectAll))
        .separator()
        .menu_with_disabled(
            t!("query.run_selected").to_string(),
            none,
            Box::new(RunSelected),
        )
        .menu(t!("query.run_current").to_string(), Box::new(RunCurrent))
        .menu_with_disabled(
            t!("query.explain_selected").to_string(),
            none,
            Box::new(Explain),
        )
        .separator()
        .menu(t!("query.beautify").to_string(), Box::new(Beautify))
        .submenu(
            t!("query.comment").to_string(),
            NativeMenu::new()
                .menu(
                    t!("query.comment_lines").to_string(),
                    Box::new(CommentLines),
                )
                .menu(
                    t!("query.uncomment_lines").to_string(),
                    Box::new(UncommentLines),
                ),
        )
        .submenu(
            t!("query.convert_case").to_string(),
            NativeMenu::new()
                .menu(t!("query.upper_case").to_string(), Box::new(UpperCase))
                .menu(t!("query.lower_case").to_string(), Box::new(LowerCase)),
        )
        .separator()
        .menu(
            t!("query.create_snippet").to_string(),
            Box::new(navidog_core::actions::CreateSnippet {
                code: selected.into(),
            }),
        )
}

fn separator(cx: &App) -> impl IntoElement {
    div()
        .w(tokens::pane::DIVIDER.rems())
        .h(tokens::navigator::ROW_ICON.rems())
        .mx_1()
        .bg(palette(cx).separator)
}

/// Plain text above the size limit or with syntax highlighting turned off (F-PREF-004).
pub(crate) fn language_for(s: &Settings, len: usize) -> &'static str {
    if !s.syntax_highlight || len > s.highlight_limit_mb as usize * 1024 * 1024 {
        "text"
    } else {
        "sql"
    }
}

fn uuid_v4() -> String {
    format!(
        "{:016x}{:016x}",
        rand_u64(),
        rand_u64() ^ std::process::id() as u64
    )
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish()
}

impl Render for QueryEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let position = Settings::global(cx).results_position;
        let results = self.render_results(window, cx).into_any_element();
        let split = Settings::global(cx).editor_split;
        let remember = |state: &Entity<gpui_kit::component::resizable::ResizableState>,
                        _: &mut Window,
                        cx: &mut App| {
            if let Some(size) = state.read(cx).sizes().first() {
                let size = f32::from(*size);
                Settings::update(cx, |s| s.editor_split = Some(size));
            }
        };
        let editor_panel = resizable_panel()
            // raw-px: the divider position the user dragged to, saved in pixels
            .when_some(split, |p, size| p.size(px(size)))
            .child(self.render_editor(cx));
        let body = match position {
            ResultsPosition::Below => v_resizable("query-split")
                .on_resize(remember)
                .child(editor_panel)
                .child(resizable_panel().child(results))
                .into_any_element(),
            ResultsPosition::Beside => h_resizable("query-split-beside")
                .on_resize(remember)
                .child(editor_panel)
                .child(resizable_panel().child(results))
                .into_any_element(),
            ResultsPosition::NewPage if self.results_page => results,
            ResultsPosition::NewPage => self.render_editor(cx).into_any_element(),
        };
        v_flex()
            .id("query-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(p.control_bg)
            .on_action(cx.listener(|this, _: &Run, window, cx| this.run(RunKind::All, window, cx)))
            .on_action(cx.listener(|this, _: &RunCurrent, window, cx| {
                this.run(RunKind::Current, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunSelected, window, cx| {
                this.run(RunKind::Selected, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &Explain, window, cx| this.run(RunKind::Explain, window, cx)),
            )
            .on_action(cx.listener(|this, _: &Stop, _, cx| this.stop(cx)))
            .on_action(|_: &ToggleContinueOnError, _, cx| {
                Settings::update(cx, |s| s.continue_on_error = !s.continue_on_error)
            })
            .on_action(|_: &ToggleKeepResults, _, cx| {
                Settings::update(cx, |s| s.keep_results = !s.keep_results)
            })
            // Result-tab menus dispatch along the focused element (usually the SQL editor)
            // and bubble to this root. Handlers on the results pane never see them.
            .on_action(
                cx.listener(|this, a: &TogglePinTab, _, cx| this.results.toggle_pin(a.id, cx)),
            )
            .on_action(
                cx.listener(|this, a: &CloseResultTab, _, cx| this.results.close_tab(a.id, cx)),
            )
            .on_action(cx.listener(|this, _: &UnpinAllTabs, _, cx| {
                let ids: Vec<u64> = this
                    .results
                    .tabs
                    .iter()
                    .filter(|t| t.pinned)
                    .map(|t| t.id)
                    .collect();
                for id in ids.into_iter().rev() {
                    this.results.toggle_pin(id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseAllResults, _, cx| {
                let ids: Vec<u64> = this.results.tabs.iter().map(|t| t.id).collect();
                for id in ids {
                    this.results.close_tab(id, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &PickConnection, window, cx| {
                this.pick_connection(a.id.to_string(), window, cx)
            }))
            .on_action(cx.listener(|this, a: &PickDatabase, _, cx| {
                this.pick_database(a.name.to_string(), cx)
            }))
            .on_action(
                cx.listener(|this, a: &PickSchema, _, cx| this.pick_schema(a.name.to_string(), cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(1., cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-1., cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| {
                this.zoom = None;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &ToggleWordWrap, window, cx| {
                    this.toggle_word_wrap(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ResultsBelow, window, cx| {
                this.place_results(ResultsPosition::Below, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ResultsBeside, window, cx| {
                this.place_results(ResultsPosition::Beside, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ResultsNewPage, window, cx| {
                this.place_results(ResultsPosition::NewPage, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::results::BackToEditor, window, cx| {
                    this.back_to_editor(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ShowResults, _, cx| {
                if this.results.has_run() {
                    this.results_page = true;
                    cx.notify();
                }
            }))
            .on_action(|_: &ShowResultAfterRun, _, cx| {
                Settings::update(cx, |s| s.show_after_run = ShowAfterRun::Result)
            })
            .on_action(|_: &ShowMessagesAfterRun, _, cx| {
                Settings::update(cx, |s| s.show_after_run = ShowAfterRun::Messages)
            })
            .on_action(|_: &ShowSummaryAfterRun, _, cx| {
                Settings::update(cx, |s| s.show_after_run = ShowAfterRun::Summary)
            })
            .on_action(cx.listener(|this, _: &ToggleProfile, _, cx| {
                if matches!(this.dialect(cx), Some(Dialect::MySql)) {
                    Settings::update(cx, |s| s.show_profile = !s.show_profile);
                }
            }))
            // Snippets and identifiers dragged from the info pane land at the caret (a recorded
            // deviation: the editor does not map a drop point to a text position)
            .on_drop(
                cx.listener(|this, a: &navidog_core::actions::InsertText, window, cx| {
                    this.insert_text(&a.text, a.snippet, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::save::SaveQuery, window, cx| {
                this.save_query(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::save::SaveQueryAs, window, cx| {
                    this.save_query_as(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::save::SaveAsExternal, window, cx| {
                    let rebind = matches!(this.source, QuerySource::Untitled(_));
                    this.save_as_external(rebind, false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::save::SaveAsNavicat, window, cx| {
                    if let Some(scope) = this.target.scope() {
                        this.save_as_navicat(scope, false, window, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &navidog_grid::ShowFind, window, cx| {
                this.open_find(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &navidog_grid::ShowReplace, window, cx| {
                    this.open_find(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &navidog_grid::FindNext, window, cx| {
                this.find_step(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::find::FindPrevious, window, cx| {
                    this.find_step(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::find::ToggleFindRegex, window, cx| {
                    this.toggle_find_option(|o| o.regex = !o.regex, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::find::ToggleFindCase, window, cx| {
                    this.toggle_find_option(|o| o.case = !o.case, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::find::ToggleFindWholeWord, window, cx| {
                    this.toggle_find_option(|o| o.whole_word = !o.whole_word, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::find::FindLiteral, window, cx| {
                    this.open_find(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::find::FindReplace, window, cx| {
                    this.open_find(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::CommentLines, window, cx| {
                    this.comment(CommentMode::Comment, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::UncommentLines, window, cx| {
                    this.comment(CommentMode::Uncomment, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::ToggleComment, window, cx| {
                    this.comment(CommentMode::Toggle, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::UpperCase, window, cx| {
                    this.change_case(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::LowerCase, window, cx| {
                    this.change_case(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::Beautify, window, cx| {
                    this.beautify(window, cx)
                }),
            )
            .on_action(
                cx.listener(|_, _: &crate::format::BeautifyOptionsDialog, window, cx| {
                    crate::format::open_beautify_options(cx.entity().downgrade(), window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::Minify, window, cx| this.minify(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &crate::format::PasteFromStack, window, cx| {
                    this.paste_from_stack(window, cx)
                }),
            )
            // Capture phase: these run before the text field handles the key
            .capture_action(cx.listener(|this, _: &Copy, window, cx| {
                if this.editor_focused(window, cx) {
                    this.remember_copy(cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &Cut, window, cx| {
                if this.editor_focused(window, cx) {
                    this.remember_copy(cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if !this.editor_focused(window, cx) {
                    return;
                }
                // Tab accepts the completion like Enter (30 §4)
                if this.completion_open(cx) {
                    if this.accepts_completion(cx) {
                        this.editor.update(cx, |e, cx| {
                            e.route_overlay_action(
                                Box::new(Enter {
                                    secondary: false,
                                    shift: false,
                                }),
                                window,
                                cx,
                            );
                        });
                        cx.stop_propagation();
                    } else {
                        this.editor
                            .update(cx, |e, cx| e.dismiss_completion_overlay(cx));
                    }
                    return;
                }
                if this.step_tab_stop(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                if this.editor_focused(window, cx)
                    && this.completion_open(cx)
                    && !this.accepts_completion(cx)
                {
                    this.editor
                        .update(cx, |e, cx| e.dismiss_completion_overlay(cx));
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.completion_open(cx) {
                    this.menu_moved = true;
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.completion_open(cx) {
                    this.menu_moved = true;
                }
            }))
            .on_action(cx.listener(|this, _: &UpdateOutline, _, cx| this.ensure_outline(true, cx)))
            .capture_action(cx.listener(|this, _: &OutdentInline, window, cx| {
                if this.editor_focused(window, cx) && this.step_tab_stop(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.tab_stops.is_some() {
                    this.end_tab_stops(cx);
                    cx.stop_propagation();
                } else if this.find.open && !this.completion_open(cx) {
                    this.close_find(window, cx);
                    cx.stop_propagation();
                } else if this.editor_focused(window, cx)
                    && !this.completion_open(cx)
                    && crate::completion::completion_on(this, cx)
                {
                    // Esc with the menu closed opens it (30 §4)
                    this.editor
                        .update(cx, |e, cx| e.show_completions(window, cx));
                    cx.stop_propagation();
                }
            }))
            .child(self.render_toolbar(cx))
            .when(self.find.open, |v| v.child(self.render_find(cx)))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
    }
}

fn set_position(position: ResultsPosition, cx: &mut App) {
    Settings::update(cx, |s| s.results_position = position);
    // Every open query editor follows; the menus show the check
    cx.refresh_windows();
}

impl QueryEditor {
    /// Result-position buttons call this directly. `dispatch_action` follows focus, and after the
    /// results page unmounts the SQL editor that focus is no longer in the tree.
    pub(crate) fn place_results(
        &mut self,
        position: ResultsPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let show_page = position == ResultsPosition::NewPage && self.results.has_run();
        self.results_page = show_page;
        set_position(position, cx);
        cx.notify();
        if !show_page {
            self.focus_sql(window, cx);
        }
    }

    pub(crate) fn back_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.results_page = false;
        cx.notify();
        self.focus_sql(window, cx);
    }

    fn focus_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.focus(window, cx));
    }
}

impl QueryEditor {
    pub(crate) fn connection_name(&self, cx: &App) -> String {
        self.target
            .connection
            .as_ref()
            .and_then(|id| ConnectionStore::global(cx).read(cx).get(id))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }
}
