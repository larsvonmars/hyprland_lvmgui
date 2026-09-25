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

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use hypr_osd_core::windows::{Window, Workspace};
use hypr_osd_core::{icons, text};

use crate::layout::Plan;
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

/// What every tile spends on its title line rather than on its picture. It is
/// part of the tile height `layout.rs` solves for, so it is written down here -
/// next to the view that draws it - and read from there.
pub const TITLE: i32 = 26;

/// Gap between the chips in the strip.
const CHIP_GAP: i32 = 8;

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

pub struct OverviewView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    strip: gtk::Box,
    rows: gtk::Box,
    tiles: RefCell<Vec<Rc<Tile>>>,
    on_tile: RefCell<Option<TileHandler>>,
    on_workspace: RefCell<Option<WorkspaceHandler>>,
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

        let hint = gtk::Label::new(Some(
            "arrows walk · Enter focuses · 1-9 switch workspace · R re-captures · Esc closes",
        ));
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

        OverviewView {
            root,
            strip,
            rows,
            tiles: RefCell::new(Vec::new()),
            on_tile: RefCell::new(None),
            on_workspace: RefCell::new(None),
        }
    }

    /// What to do when a tile is clicked, with the tile's index.
    pub fn on_tile_click(&self, handler: impl Fn(usize) + 'static) {
        *self.on_tile.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a workspace chip is clicked, with the workspace's name.
    pub fn on_workspace_click(&self, handler: impl Fn(&str) + 'static) {
        *self.on_workspace.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the card itself is clicked - anywhere that is not a tile
    /// or a chip. A full-screen card is a place you leave, and the quickest way
    /// out of one is to click the part of it that is not a thing you can press.
    ///
    /// The buttons claim their own clicks, so this only ever fires for the rest:
    /// a gesture that loses the sequence to a button does not report a release.
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
                let tile = Rc::new(Tile::new(window, width, picture, settings.icon_size));
                if let Some(on_tile) = self.on_tile.borrow().clone() {
                    let index = *index;
                    tile.button.connect_clicked(move |_| on_tile(index));
                }
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
        for (position, tile) in self.tiles.borrow().iter().enumerate() {
            if position == index {
                tile.button.add_css_class("selected");
            } else {
                tile.button.remove_css_class("selected");
            }
        }
    }

    /// Everything the last render put on the card. The tiles go with the rows
    /// that hold them; the list of them is kept because the element needs to
    /// select one and to hand a capture to its tile.
    fn clear(&self) {
        self.tiles.borrow_mut().clear();
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
                chip.connect_clicked(move |_| on_workspace(&name));
            }
            self.strip.append(&chip);
        }
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
