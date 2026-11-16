//! The overview's widgets: a strip of workspaces over a field of window tiles.
//!
//! The card is a *view*: it draws what it is handed and reports what was
//! clicked. How large a tile is, and which rows they fall into, is `layout.rs`'s
//! business; what a click or a key *does* is `main.rs`'s. Nothing here knows how
//! to focus a window.
//!
//! A tile is the window's own picture - a capture, see `thumbs` - with its
//! workspace on it and its title under it. Where there is no picture to be had,
//! the tile keeps the application's icon instead, in a frame that says "missing"
//! rather than "empty": a window whose capture failed is still a window you want
//! to be able to see and switch to.
//!
//! # Dragging a window onto a workspace
//!
//! A tile can be dragged onto a workspace chip - or onto another tile, which
//! means "wherever that one lives" - and the drop says where the window belongs
//! ([`Drop`], handed to [`OverviewView::on_drop`]). The card is a picture of the
//! desktop you can *rearrange*, which is the one thing a workspace overview is
//! for that a row of thumbnails is not.
//!
//! Every tile carries one `GtkGestureDrag`, in the **capture** phase, and that
//! is deliberate: a drag gesture *claims the press* it starts from (`GtkDragSource`
//! has to give it back for exactly this reason) - and the tile is a `GtkButton`,
//! whose own click gesture claims the press in the bubble phase and would deny a
//! gesture on a *parent* the motion that follows. Claiming first also means the
//! button never activates, so the tiles are no longer buttons that answer clicks
//! themselves: a release with the pointer still where it was pressed is the click
//! ([`OverviewView::on_tile_click`] fires), and a release anywhere else is a drop.
//!
//! Everything a drop can land on is measured *once*, when the drag begins: the
//! chips and the tiles are exactly where they were, and a drag cannot move
//! anything until it is released. `layout::under` then answers which of them the
//! pointer is over, and that is a pure function with a test.
//!
//! # By finger
//!
//! Everything here can be pressed with a fingertip already - a tile is a button,
//! a workspace is a chip, and the background closes the card - so the touch work
//! is about *size* and about the one action that had no button: taking the
//! pictures again. On this card that is the `R` key, which a touchscreen does not
//! have, so the strip grows a "Recapture" chip ([`OverviewView::on_recapture_click`])
//! that does the same thing. Under `.touch` the chips and the hint line are
//! finger-sized and the strip is given a taller row ([`TOUCH_STRIP`]), which
//! `main.rs` subtracts from the tile area exactly like the pointer-sized one.
//! Dragging works with a finger for the same reason it works with a mouse:
//! `GtkGestureDrag` speaks both, so a window can be pushed onto a workspace chip
//! by hand.
//!
//! The card also reports *every* press and scroll as activity
//! ([`OverviewView::on_activity`]): a finger leaves no pointer motion to watch,
//! and the overview's idle-close watchdog must not take the desktop away while
//! somebody is looking at it and touching its tiles.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use hypr_osd_core::windows::{Window, Workspace};
use hypr_osd_core::{icons, text};

use crate::layout::{self, Plan, Span};
use crate::Settings;

/// Air between the card's edge and the strip, the tiles and the hint. The shell
/// adds the card's own padding (CARD_PAD_*) around the content widget, and
/// `main.rs` subtracts both when it works out how much room the tiles have - so
/// the number is written down once, here, and used from there.
pub const PADDING: i32 = 22;

/// The height the strip of workspaces and the hint line take. They are *given*
/// these heights rather than measured, because `main.rs` subtracts them from the
/// screen when it sizes the tiles: a row that does not fit is a row nobody can
/// see, and font metrics are not something to bet a whole row of windows on.
pub const STRIP: i32 = 32;
pub const HINT: i32 = 20;

/// The same strip on a tablet: `button.chip` becomes a 44px target under `.touch`
/// (24px of content plus 10px of padding a side, see `overview.css`), and the
/// strip is given the extra two pixels so a chip's focus ring cannot clip.
pub const TOUCH_STRIP: i32 = 46;

/// Every tile spends this on its title line rather than on its picture. It is
/// part of the tile height `layout.rs` solves for, so it is written down here -
/// next to the view that draws it - and read from there.
pub const TITLE: i32 = 26;

/// Gap between the chips in the strip.
const CHIP_GAP: i32 = 8;

/// How far the pointer has to travel before a press on a tile is a *drag* rather
/// than a click on it. Eight pixels is GTK's own drag threshold on a pointer
/// device, and it is also about as far as a fingertip rolls while it touches.
const DRAG_THRESHOLD: f64 = 8.0;

/// The hint line on a machine with a pointer: the keys are not obvious from
/// looking at the card, so it says them.
const HINT_POINTER: &str =
    "arrows walk · Enter focuses · drag a tile onto a workspace to move it · \
     1-9 switch · R re-captures · Esc closes";

/// And the hint on a tablet, where the only ways in are a tap on a tile, a tap on
/// a chip, or the Recapture chip - and the way out is the empty space around them.
const HINT_TOUCH: &str =
    "drag a window onto a workspace to move it · tap to go there · tap outside to close";

/// A tile's own padding and border, from this element's CSS. Subtracted from the
/// width `layout.rs` planned, so that a tile ends up exactly as wide as the plan
/// assumed when it packed the rows.
const TILE_PAD: i32 = 8;
const TILE_BORDER: i32 = 1;
const TILE_INNER: i32 = TILE_PAD + TILE_BORDER;

/// Space between a tile's picture and its title, and between picture and badge.
const TILE_GAP: i32 = 6;

/// What a click on a tile does, with the tile's index.
type TileHandler = Rc<dyn Fn(usize)>;
/// What a click on a workspace chip does, with the workspace's name.
type WorkspaceHandler = Rc<dyn Fn(&str)>;
/// What a click on the Recapture chip does.
type RecaptureHandler = Rc<dyn Fn()>;
/// What a dropped tile means: the index of the tile that was dragged, and the
/// thing it was dropped on.
type DropHandler = Rc<dyn Fn(usize, Drop)>;

/// What a dragged tile was dropped on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    /// A workspace chip in the strip: the window belongs on that workspace.
    Workspace(String),
    /// Another window's tile, i.e. "wherever that one lives".
    Window(usize),
}

/// One drag in progress.
///
/// Everything here is in the *rows'* coordinates - the widget the gesture is
/// attached to - which is also where `start` and the offsets are measured, so no
/// coordinate has to be converted while the pointer moves.
struct Dragging {
    /// The index of the tile that was pressed.
    from: usize,
    /// Where the press was.
    start: (f64, f64),
    /// Whether the pointer has left the click's neighbourhood yet. A release
    /// without this is a click on the tile, not a drop.
    moving: bool,
    /// Everything a drop can land on, measured when the drag began.
    targets: Vec<(Span, Drop)>,
    /// Which of `targets` wears the highlight right now.
    lit: Option<usize>,
}

pub struct OverviewView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    strip: gtk::Box,
    rows: gtk::Box,
    hint: gtk::Label,
    /// The tiles in drawing order, which is also the order of the windows they
    /// stand for: the plan lays every window out exactly once.
    tiles: Rc<RefCell<Vec<Rc<Tile>>>>,
    /// The workspace chips, with the workspace each one stands for - what a drop
    /// on one of them means.
    chips: Rc<RefCell<Vec<Chip>>>,
    /// Which tile wears the selection. Kept so that walking the tiles only
    /// touches the two that changed: this runs on every arrow key, and it used
    /// to give every tile on the card a new CSS class and take it away again.
    selected: Rc<Cell<Option<usize>>>,
    on_tile: Rc<RefCell<Option<TileHandler>>>,
    on_workspace: Rc<RefCell<Option<WorkspaceHandler>>>,
    on_recapture: Rc<RefCell<Option<RecaptureHandler>>>,
    on_drop: Rc<RefCell<Option<DropHandler>>>,
}

/// One workspace chip in the strip.
struct Chip {
    button: gtk::Button,
    /// The workspace's own name, which is what a drop on the chip means.
    name: String,
}

impl OverviewView {
    pub fn new(settings: &Settings) -> Self {
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, CHIP_GAP);
        strip.set_size_request(-1, STRIP);
        strip.set_halign(gtk::Align::Center);
        strip.set_valign(gtk::Align::Start);

        // The tiles live in rows built by `render`, not in a Grid: a row is a
        // widget, which is what makes it a row the arrow keys can walk and the
        // card can centre without measuring anything.
        //
        // No `vexpand` here, and that matters: a box with room to spare hands the
        // slack to its children, which stretches every tile and pulls the
        // pictures off the sizes the plan solved for. Left to its own height and
        // centred by `valign`, the block of rows keeps them exactly.
        let rows = gtk::Box::new(gtk::Orientation::Vertical, settings.gap);
        rows.set_valign(gtk::Align::Center);

        let hint = gtk::Label::new(Some(HINT_POINTER));
        hint.add_css_class("hint");
        hint.set_size_request(-1, HINT);
        hint.set_valign(gtk::Align::End);

        let root = gtk::Box::new(gtk::Orientation::Vertical, settings.gap);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_margin_start(PADDING);
        root.set_margin_end(PADDING);
        root.set_margin_top(PADDING);
        root.set_margin_bottom(PADDING);
        root.append(&strip);
        root.append(&rows);
        root.append(&hint);

        let tiles: Rc<RefCell<Vec<Rc<Tile>>>> = Rc::new(RefCell::new(Vec::new()));
        let chips: Rc<RefCell<Vec<Chip>>> = Rc::new(RefCell::new(Vec::new()));
        let selected = Rc::new(Cell::new(None));
        let on_tile: Rc<RefCell<Option<TileHandler>>> = Rc::new(RefCell::new(None));
        let on_workspace: Rc<RefCell<Option<WorkspaceHandler>>> = Rc::new(RefCell::new(None));
        let on_recapture: Rc<RefCell<Option<RecaptureHandler>>> = Rc::new(RefCell::new(None));
        let on_drop: Rc<RefCell<Option<DropHandler>>> = Rc::new(RefCell::new(None));

        OverviewView {
            root,
            strip,
            rows,
            hint,
            tiles,
            chips,
            selected,
            on_tile,
            on_workspace,
            on_recapture,
            on_drop,
        }
    }

    /// The height the workspace strip needs in this mode - what `main.rs`
    /// subtracts from the screen when it works out the room the tiles have.
    pub fn strip_height(touch: bool) -> i32 {
        if touch {
            TOUCH_STRIP
        } else {
            STRIP
        }
    }

    /// Follow the machine: on a tablet the chips and the hint grow, the strip
    /// gets the room the bigger chips need, and the hint says what a finger can
    /// do instead of naming keys that are not there.
    ///
    /// The tiles are *not* re-planned here - that is `main.rs`'s business,
    /// because the strip height is part of the arithmetic - but a card that is
    /// already on screen must not be left with a strip taller than the room it
    /// was planned for, which is why the element re-opens in that case.
    pub fn apply_touch(&self, tablet: bool) {
        self.strip.set_size_request(-1, Self::strip_height(tablet));
        self.hint
            .set_text(if tablet { HINT_TOUCH } else { HINT_POINTER });
    }

    /// What to do when a tile is clicked, with the tile's index.
    pub fn on_tile_click(&self, handler: impl Fn(usize) + 'static) {
        *self.on_tile.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a workspace chip is clicked, with the workspace's name.
    pub fn on_workspace_click(&self, handler: impl Fn(&str) + 'static) {
        *self.on_workspace.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the Recapture chip is clicked. The touch path for the `R`
    /// key: taking the pictures again is the card's one action that is not
    /// reachable by pointing at the thing it acts on.
    pub fn on_recapture_click(&self, handler: impl Fn() + 'static) {
        *self.on_recapture.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a tile is dropped on something: the index of the tile that
    /// was dragged, and what it landed on.
    ///
    /// The view reports the gesture and nothing else - which window that index
    /// stands for, and what moving it means, is `main.rs`'s business.
    pub fn on_drop(&self, handler: impl Fn(usize, Drop) + 'static) {
        *self.on_drop.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the card itself is clicked - anywhere that is not a tile
    /// or a chip. A full-screen card is a place you leave, and the quickest way
    /// out of one is to click the part of it that is not a thing you can press.
    ///
    /// The tiles and chips claim their own presses, so this only ever fires for
    /// the rest. That is also why it is a *release* and not a press: a press
    /// outside a tile must still be allowed to become a drag on the card.
    pub fn on_background_click(&self, handler: impl Fn() + 'static) {
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |_, _, _, _| handler());
        self.root.add_controller(gesture);
    }

    /// What to do when the pointer moves over the card. The element uses it to
    /// tell "somebody is looking at this" from "nobody is here": a card that
    /// holds the keyboard must not sit there holding it because its owner walked
    /// away.
    pub fn on_motion(&self, handler: impl Fn() + 'static) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, _, _| handler());
        self.root.add_controller(motion);
    }

    /// What to do when a finger touches the card at all - a press, a release or a
    /// scroll.
    ///
    /// A touchscreen produces no pointer motion until the finger lands, so the
    /// motion controller above sees nothing while somebody is reading the card
    /// with a hand on it, and the idle-close watchdog would take the desktop away
    /// mid-look. This is a *legacy* controller in the capture phase: it observes
    /// every event without claiming it (a `GestureClick` here would race the tile
    /// buttons and could eat their taps) and passes it on unchanged.
    pub fn on_activity(&self, handler: impl Fn() + 'static) {
        let handler = Rc::new(handler);
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        events.connect_event(move |_, event| {
            // Button *and* touch spellings: a finger is reported as a touch
            // sequence and, when nothing claimed it, as an emulated button too -
            // either one is somebody looking at the card.
            if matches!(
                event.event_type(),
                gdk::EventType::ButtonPress
                    | gdk::EventType::ButtonRelease
                    | gdk::EventType::TouchBegin
                    | gdk::EventType::TouchUpdate
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel
                    | gdk::EventType::Scroll
            ) {
                handler();
            }
            gtk::glib::Propagation::Proceed
        });
        self.root.add_controller(events);
    }

    /// Draw `plan`: the workspaces on top, then the rows of tiles.
    pub fn render(
        &self,
        plan: &Plan,
        windows: &[Window],
        workspaces: &[Workspace],
        focused: &str,
        settings: &Settings,
    ) {
        self.clear();
        self.render_strip(workspaces, focused);

        let mut tiles = self.tiles.borrow_mut();
        let picture = (plan.tile_height - TITLE).max(1);
        for row in &plan.rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, settings.gap);
            line.set_halign(gtk::Align::Center);
            for index in row {
                let Some(window) = windows.get(*index) else {
                    // Cannot happen - the plan was built from this very list -
                    // but stopping is still better than drawing tiles whose
                    // indices no longer match the windows they stand for.
                    eprintln!("hypr-osd-overview: the plan does not match the window list");
                    return;
                };
                let width = plan.widths.get(*index).copied().unwrap_or(240);
                // No click handler on the tile: the drag gesture below claims
                // the press and reports the click it turns out to be (see the
                // module comment), so a `clicked` handler here would either
                // never fire or fire a second time.
                let tile = Rc::new(Tile::new(window, width, picture, settings.icon_size));
                attach_drag(
                    &tile,
                    *index,
                    &self.tiles,
                    &self.chips,
                    &self.on_tile,
                    &self.on_drop,
                );
                line.append(&tile.button);
                tiles.push(tile);
            }
            self.rows.append(&line);
        }
    }

    /// The tiles, in the order they were drawn - which is the order the windows
    /// were handed over in. `thumbs` fills their pictures in.
    pub fn tiles(&self) -> Vec<Rc<Tile>> {
        self.tiles.borrow().clone()
    }

    /// Put the accent on one tile.
    pub fn select(&self, index: usize) {
        if self.selected.get() == Some(index) {
            return;
        }
        let tiles = self.tiles.borrow();
        if let Some(previous) = self.selected.replace(Some(index)) {
            if let Some(tile) = tiles.get(previous) {
                tile.button.remove_css_class("selected");
            }
        }
        if let Some(tile) = tiles.get(index) {
            tile.button.add_css_class("selected");
        }
    }

    /// Everything the last render put on the card. The tiles go with the rows
    /// that hold them; the list of them is kept because the element needs to
    /// select one and to hand a capture to its tile.
    fn clear(&self) {
        self.tiles.borrow_mut().clear();
        self.chips.borrow_mut().clear();
        // A render replaces every tile, so the one that wore the selection went
        // with them.
        self.selected.set(None);
        for container in [&self.rows, &self.strip] {
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
        }
    }

    fn render_strip(&self, workspaces: &[Workspace], focused: &str) {
        for workspace in workspaces {
            let chip = gtk::Button::with_label(&workspace.name);
            chip.add_css_class("chip");
            chip.set_tooltip_text(Some(&describe(workspace)));
            if workspace.name == focused {
                // Where you are - the only thing on this card that still answers
                // that, with the whole desktop behind it.
                chip.add_css_class("current");
            }
            let name = workspace.name.clone();
            if let Some(on_workspace) = self.on_workspace.borrow().clone() {
                let switch_to = name.clone();
                chip.connect_clicked(move |_| on_workspace(&switch_to));
            }
            self.chips.borrow_mut().push(Chip {
                button: chip.clone(),
                name,
            });
            self.strip.append(&chip);
        }

        // The `R` key's hand: a finger has no way to say "take the pictures
        // again" otherwise, and it is the one action on this card that is not
        // attached to a thing you can point at. It sits at the end of the strip
        // because it is a card-wide action rather than a workspace.
        let recapture = gtk::Button::new();
        recapture.add_css_class("chip");
        recapture.add_css_class("action");
        recapture.set_tooltip_text(Some("Take the pictures again (R)"));
        let icon = icons::lucide(hypr_osd_core::icons::names::ROTATE_CW, 14);
        let label = gtk::Label::new(Some("Recapture"));
        label.add_css_class("action-label");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&icon);
        content.append(&label);
        recapture.set_child(Some(&content));
        if let Some(on_recapture) = self.on_recapture.borrow().clone() {
            recapture.connect_clicked(move |_| on_recapture());
        }
        self.strip.append(&recapture);
    }
}

/// One window, as a tile: its picture (or what stands in for one), the workspace
/// it lives on, and its title.
pub struct Tile {
    button: gtk::Button,
    /// Switched from the placeholder to the picture once a capture arrives.
    art: gtk::Stack,
    picture: gtk::Picture,
}

impl Tile {
    fn new(window: &Window, width: i32, picture: i32, icon_size: i32) -> Self {
        let art_width = (width - 2 * TILE_INNER).max(1);

        let image = gtk::Picture::new();
        image.set_content_fit(gtk::ContentFit::Contain);
        image.set_can_shrink(true);

        let placeholder = placeholder(window, picture, icon_size);

        let art = gtk::Stack::new();
        art.set_size_request(art_width, picture);
        art.add_named(&placeholder, Some("placeholder"));
        art.add_named(&image, Some("picture"));
        art.set_visible_child_name("placeholder");

        // The workspace a window lives on, written on the picture itself: the
        // tiles are grouped by workspace, but a row of them does not say which
        // one it came from.
        let badge = gtk::Label::new(Some(&window.workspace));
        badge.add_css_class("ws-badge");
        badge.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text::cap_width(&badge, (width / 2).max(24));
        badge.set_halign(gtk::Align::Start);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_start(TILE_PAD);
        badge.set_margin_top(TILE_PAD);

        let framed = gtk::Overlay::new();
        framed.set_child(Some(&art));
        framed.add_overlay(&badge);

        let title = gtk::Label::new(Some(window.label()));
        title.add_css_class("tile-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_justify(gtk::Justification::Center);
        text::cap_width(&title, width - 2 * TILE_INNER);

        let content = gtk::Box::new(gtk::Orientation::Vertical, TILE_GAP);
        content.append(&framed);
        content.append(&title);

        let button = gtk::Button::new();
        button.add_css_class("tile");
        button.set_child(Some(&content));
        button.set_size_request(width, -1);
        button.set_tooltip_text(Some(&window.describe()));

        Tile {
            button,
            art,
            picture: image,
        }
    }

    /// Show the capture that arrived.
    pub fn set_picture(&self, texture: &gdk::Texture) {
        self.picture.set_paintable(Some(texture));
        self.art.set_visible_child_name("picture");
    }
}

/// What stands where a picture would be: the application's icon, or its first
/// letter, and the class underneath.
fn placeholder(window: &Window, picture: i32, icon_size: i32) -> gtk::Box {
    // Room for the class name under the icon, and never so large that the icon
    // takes the whole tile and pushes it out.
    let size = icon_size.clamp(16, (picture - 24).max(16));

    let icon = gtk::Image::new();
    icon.set_pixel_size(size);
    icon.set_halign(gtk::Align::Center);

    let letter = gtk::Label::new(Some(&text::initial(&window.class)));
    letter.add_css_class("icon-letter");
    letter.set_size_request(size, size);

    let mark = gtk::Stack::new();
    mark.set_halign(gtk::Align::Center);
    mark.add_named(&icon, Some("icon"));
    mark.add_named(&letter, Some("letter"));
    mark.set_visible_child_name("letter");
    if let Some(paintable) = icons::paintable(&window.class, size) {
        icon.set_paintable(Some(&paintable));
        mark.set_visible_child_name("icon");
    }

    let class = gtk::Label::new(Some(if window.class.is_empty() {
        "no picture"
    } else {
        &window.class
    }));
    class.add_css_class("placeholder-class");
    class.set_ellipsize(gtk::pango::EllipsizeMode::End);
    class.set_halign(gtk::Align::Center);
    text::cap_width(&class, 160);

    let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 4);
    placeholder.add_css_class("placeholder");
    placeholder.set_halign(gtk::Align::Fill);
    placeholder.set_valign(gtk::Align::Fill);
    mark.set_valign(gtk::Align::End);
    mark.set_vexpand(true);
    placeholder.append(&mark);
    placeholder.append(&class);
    placeholder
}

/// A workspace chip's tooltip.
fn describe(workspace: &Workspace) -> String {
    match workspace.windows {
        0 => format!("workspace {} · empty", workspace.name),
        1 => format!("workspace {} · 1 window", workspace.name),
        count => format!("workspace {} · {count} windows", workspace.name),
    }
}

/// Give one tile its pointer gesture: the press that turns into either a click on
/// that tile or a drag of it onto a workspace.
///
/// The gesture sits on the tile and in the **capture** phase, and it *claims the
/// press* itself. That combination is what makes it work at all: the tile is a
/// `GtkButton`, whose own click gesture claims the press in the bubble phase and
/// would otherwise deny a gesture on a parent the motion that follows it. Claiming
/// it first also means the button itself never activates, so a click on a tile is
/// only ever what this gesture says it is.
///
/// Everything a drop can land on is measured when the drag begins, in the tile's
/// own coordinates - the space the gesture reports its start point and offsets
/// in, and a space that does not move while a tile is being carried. `layout::under`
/// then answers which target the pointer is over, which is a pure function and
/// the reason the arithmetic has a test.
fn attach_drag(
    tile: &Rc<Tile>,
    index: usize,
    tiles: &Rc<RefCell<Vec<Rc<Tile>>>>,
    chips: &Rc<RefCell<Vec<Chip>>>,
    on_tile: &Rc<RefCell<Option<TileHandler>>>,
    on_drop: &Rc<RefCell<Option<DropHandler>>>,
) {
    let state: Rc<RefCell<Option<Dragging>>> = Rc::new(RefCell::new(None));
    let gesture = gtk::GestureDrag::new();
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);

    // The press is ours from here on: the button cannot take it, so one gesture
    // decides whether the release is a click or a drop.
    gesture.connect_drag_begin({
        let button = tile.button.clone();
        let tiles = tiles.clone();
        let chips = chips.clone();
        let state = state.clone();
        move |gesture, start_x, start_y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let tiles = tiles.borrow();
            let chips = chips.borrow();
            let mut targets = Vec::with_capacity(chips.len() + tiles.len());
            for chip in chips.iter() {
                if let Some(span) = span_of(&chip.button, &button) {
                    targets.push((span, Drop::Workspace(chip.name.clone())));
                }
            }
            for (other, tile) in tiles.iter().enumerate() {
                if let Some(span) = span_of(&tile.button, &button) {
                    targets.push((span, Drop::Window(other)));
                }
            }
            *state.borrow_mut() = Some(Dragging {
                from: index,
                start: (start_x, start_y),
                moving: false,
                targets,
                lit: None,
            });
        }
    });

    // The walk: once the pointer has left the click's neighbourhood, dim the tile
    // that is being carried and light whatever is under it.
    gesture.connect_drag_update({
        let tiles = tiles.clone();
        let chips = chips.clone();
        let state = state.clone();
        move |_, offset_x, offset_y| {
            let mut slot = state.borrow_mut();
            let Some(drag) = slot.as_mut() else {
                return;
            };
            if !drag.moving {
                if offset_x.hypot(offset_y) < DRAG_THRESHOLD {
                    return;
                }
                drag.moving = true;
                if let Some(tile) = tiles.borrow().get(drag.from) {
                    tile.button.add_css_class("dragging");
                }
            }

            let point = (drag.start.0 + offset_x, drag.start.1 + offset_y);
            let spans: Vec<Span> = drag.targets.iter().map(|(span, _)| *span).collect();
            let lit = layout::under(point, &spans);
            if lit == drag.lit {
                return;
            }
            if let Some(previous) = drag.lit {
                light(
                    &drag.targets[previous],
                    &tiles.borrow(),
                    &chips.borrow(),
                    false,
                );
            }
            if let Some(current) = lit {
                light(
                    &drag.targets[current],
                    &tiles.borrow(),
                    &chips.borrow(),
                    true,
                );
            }
            drag.lit = lit;
        }
    });

    // The release: a drop where the pointer is, or a click where it was pressed.
    gesture.connect_drag_end({
        let tiles = tiles.clone();
        let chips = chips.clone();
        let on_tile = on_tile.clone();
        let on_drop = on_drop.clone();
        let state = state.clone();
        move |_, _, _| {
            let Some(drag) = state.borrow_mut().take() else {
                return;
            };
            if !drag.moving {
                // The press belonged to this gesture, so a plain click on a tile
                // arrives here and nowhere else - and it means exactly what
                // pressing Enter on that tile means.
                if let Some(handler) = on_tile.borrow().clone() {
                    handler(drag.from);
                }
                return;
            }
            if let Some(tile) = tiles.borrow().get(drag.from) {
                tile.button.remove_css_class("dragging");
            }
            // A release over nothing that means anything is a cancelled drag: the
            // card stays up, because you did not ask to leave it.
            let Some(lit) = drag.lit else {
                return;
            };
            light(&drag.targets[lit], &tiles.borrow(), &chips.borrow(), false);
            if let Some(handler) = on_drop.borrow().clone() {
                // Deferred by one idle turn: a drop repaints the card, and the
                // tile it repaints is the very widget this signal is being
                // emitted from - a widget must not be torn down inside its own
                // signal (the Recapture chip has the same rule, for the same
                // reason).
                let (from, target) = (drag.from, drag.targets[lit].1.clone());
                gtk::glib::idle_add_local_once(move || handler(from, target));
            }
        }
    });

    // The sequence taken away from us (the compositor can do that): put the card
    // back the way it was, and forget the drag.
    gesture.connect_cancel({
        let tiles = tiles.clone();
        let chips = chips.clone();
        let state = state.clone();
        move |_, _| {
            let Some(drag) = state.borrow_mut().take() else {
                return;
            };
            if let Some(tile) = tiles.borrow().get(drag.from) {
                tile.button.remove_css_class("dragging");
            }
            if let Some(lit) = drag.lit {
                light(&drag.targets[lit], &tiles.borrow(), &chips.borrow(), false);
            }
        }
    });

    tile.button.add_controller(gesture);
}

/// One widget's rectangle in `target`'s coordinates, as something a drop can land
/// on. `None` for a widget GTK has not placed yet.
fn span_of(widget: &impl IsA<gtk::Widget>, target: &impl IsA<gtk::Widget>) -> Option<Span> {
    let rect = widget.compute_bounds(target)?;
    Some(Span::from_rect(
        rect.x().into(),
        rect.y().into(),
        rect.width().into(),
        rect.height().into(),
    ))
}

/// Put the drop highlight on one measured target, or take it off again.
fn light(target: &(Span, Drop), tiles: &[Rc<Tile>], chips: &[Chip], on: bool) {
    let widget = match &target.1 {
        Drop::Workspace(name) => chips
            .iter()
            .find(|chip| &chip.name == name)
            .map(|chip| chip.button.clone().upcast::<gtk::Widget>()),
        Drop::Window(index) => tiles
            .get(*index)
            .map(|tile| tile.button.clone().upcast::<gtk::Widget>()),
    };
    let Some(widget) = widget else {
        return;
    };
    if on {
        widget.add_css_class("drop-target");
    } else {
        widget.remove_css_class("drop-target");
    }
}
