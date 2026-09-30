//! The island panel: what is playing, what is waiting, and what day it is.
//!
//! Three columns of information in one card that unfolds from the bar's clock:
//!
//! ```text
//!  ┌───────────── handle ─────────────┐
//!  │ 16:04                    [playing] [hidden]  │
//!  │ Friday, 25 September   ─────────────────────────  │
//!  │ ┌─ now playing ─────┐ ┌─ calendar ─────────────┐ │
//!  │ │ ┌────┐ First Track│ │ ‹ September 2026 ›     │ │
//!  │ │ │art │ Beyoncé    │ │ Mo Tu We Th Fr Sa Su   │ │
//!  │ │ └────┘ Test Album │ │ …                      │ │
//!  │ │ ▁▁▁▁▁▁▁ 0:10 5:00  │ │                        │ │
//!  │ │ [⇄] [‹] [▶] [›] [↻]│ │                        │ │
//!  │ │ 🔊 ▁▁▁▁▁▁▁▁ 100%   │ │                        │ │
//!  │ │ [vlc] [firefox]    │ │                        │ │
//!  │ └───────────────────┘ └────────────────────────┘ │
//!  │ ┌─ notifications ──────────────────┐            │
//!  │ │ 3 unread notifications           │            │
//!  │ │ ▣ Firefox                     now│            │
//!  │ │   Test summary here              │            │
//!  │ │ ▣ Signal                   12 min│            │
//!  │ │   Another summary                │            │
//!  │ │ +1 more                          │            │
//!  │ │ [DND off] [Open]                 │            │
//!  │ │ [Hide] [Clear]                   │            │
//!  │ └──────────────────────────────────┘            │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! The view paints what it is handed and runs the commands its buttons mean.
//! Reading is `main`'s business - the notification feed arrives on its own (both
//! halves of it, see `notify`), the clock ticks, and `main`'s heartbeat calls
//! [`IslandView::refresh_media`] - with one deliberate exception: the media tile
//! reads the player itself, because two of its controls change what it shows from
//! the inside. A click on the progress bar seeks, and a click on a player chip
//! follows another player; only the tile knows either happened, so the tile is
//! what reads again (and `main` reads *the tile* when it paints the header chip).
//!
//! One detail follows from the hover design: the panel's width is what its hot
//! zone is measured against, so a widget that wants more room than it was given
//! is a bug and not a layout surprise. Every label a track or a notification can
//! fill in is therefore capped with [`text::cap_width`].
//!
//! The card itself - the fill, the border, the shadow, the 16px radius - is the
//! shell's (`box.card` in `base.css`), because this panel is a card like every
//! OSD in the collection; what is here is only what goes inside one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::icons::names;
use hypr_osd_core::mpris::{self, Direction, Playback, Player, Repeat};
use hypr_osd_core::timer::Timer;
use hypr_osd_core::{art, icons, text};

use crate::calendar::Month;
use crate::notify::{self, History, Item, Notifications};
use crate::Settings;

/// How large the panel's own marks are drawn. Two sizes, because they answer
/// two questions: a heading's or a chip's icon sits beside a 10px word and is
/// part of it, while the transport and calendar buttons are what the pointer
/// aims at.
const SMALL_ICON: i32 = 12;
const BUTTON_ICON: i32 = 14;

/// The cover: a square tile at the same 12px radius as every other nested
/// surface, with the stand-in drawn on it when a player reports no artwork (or
/// artwork that cannot be fetched).
const ART: i32 = 48;
const ART_ICON: i32 = 22;
/// The gap between the cover and the two lines of text beside it.
const ART_GAP: i32 = 10;
/// What one player chip may ask for, in pixels.
const PLAYER_CHIP: i32 = 72;
/// How long a volume drag has to settle before it reaches the player. A drag
/// asks for a new level every pixel, and every one of them would be a
/// `playerctl` process.
const VOLUME_SETTLE: Duration = Duration::from_millis(180);

/// The numbers a notification row is measured against. The first two are the
/// horizontal padding of `box.tile` and `box.notif-row` in `island.css`, and the
/// third is the icon badge plus the gap beside it: a row's text column is what is
/// left of the panel's left column once they are taken off.
const TILE_PAD_X: i32 = 10;
const ROW_PAD_X: i32 = 6;
const ROW_ART: i32 = 24 + 8;
/// The width of the age at the end of a row's first line - `59 min` is the
/// longest it ever gets.
const ROW_AGE: i32 = 38;

/// The weekday headings, Monday first - the same order [`Month::weeks`] lays the
/// grid out in.
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

pub struct IslandView {
    /// The panel's content: the card's child.
    pub root: gtk::Box,
    clock: gtk::Label,
    date: gtk::Label,
    chips: gtk::Box,
    /// The media tile: the cover and the two lines beside it, the progress bar,
    /// the five transport buttons, the player's own volume and the chips that
    /// choose which player this is.
    art: gtk::Stack,
    cover: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    progress: gtk::ProgressBar,
    elapsed: gtk::Label,
    total: gtk::Label,
    /// The progress bar and the times around it: one click target, because three
    /// pixels of bar is not something to aim at.
    seek: gtk::Box,
    play: gtk::Button,
    shuffle: gtk::Button,
    repeat: gtk::Button,
    transport: gtk::Box,
    /// The player's *own* volume - a row of its own, and hidden when the player
    /// reports none: not every player has one, and this is not the sink's.
    volume_row: gtk::Box,
    volume: gtk::Scale,
    volume_value: gtk::Label,
    /// The chips that choose the player. Empty, and hidden, unless something
    /// else is running.
    players_row: gtk::Box,
    /// What the tile is showing, and which player its buttons are aimed at -
    /// held as shared handles because they are what a *click* inside the tile
    /// changes, and the handlers are built before this view exists.
    playback: Rc<RefCell<Option<Playback>>>,
    target: Rc<RefCell<Player>>,
    /// The players that can be controlled, re-read on the caller's slower clock
    /// (`read_players` in [`IslandView::refresh_media`]).
    players: RefCell<Vec<String>>,
    /// `mpris:artUrl` currently on the cover, so a slow download cannot paint
    /// itself over the track that replaced it.
    art_url: RefCell<String>,
    /// Whether a volume drag is in flight. While it is, the slider belongs to the
    /// pointer: the poll may not pull it back under the finger, and the level the
    /// drag settles on is what reaches the player a moment later (see
    /// [`VOLUME_SETTLE`]).
    settling: Rc<Cell<bool>>,
    notif_count: gtk::Label,
    /// Where the notification rows are built.
    notif_rows: gtk::Box,
    /// The age label of every row, with the notification it belongs to: the one
    /// part of a row that has to change while nothing else does.
    ages: RefCell<Vec<(gtk::Label, Item)>>,
    /// How many rows the tile draws, and how wide the column they sit in is.
    rows: usize,
    column_width: i32,
    notif_actions: gtk::Box,
    dnd: gtk::Button,
    hide: gtk::Button,
    clear: gtk::Button,
    calendar_title: gtk::Label,
    calendar_grid: gtk::Grid,
    month: RefCell<Month>,
}

impl IslandView {
    pub fn new(settings: &Rc<Settings>) -> Rc<IslandView> {
        // The two handles a click inside the media tile reaches: which player the
        // tile follows, and the snapshot it is drawn from. They exist before the
        // widgets because the widgets' handlers capture them - and they are held
        // as shared handles rather than as fields for the same reason (the fields
        // only exist once the whole view does).
        let target: Rc<RefCell<Player>> = Rc::new(RefCell::new(Player::Active));
        let playback: Rc<RefCell<Option<Playback>>> = Rc::new(RefCell::new(None));
        let settle: Rc<Timer> = Rc::new(Timer::new());
        let settling: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        // ---- header: the clock, the date, and the state chips --------------
        let clock = gtk::Label::new(None);
        clock.add_css_class("island-clock");
        clock.set_xalign(0.0);
        let date = gtk::Label::new(None);
        date.add_css_class("island-date");
        date.set_xalign(0.0);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.append(&clock);
        heading.append(&date);

        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("chips");
        chips.set_valign(gtk::Align::Center);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("island-header");
        header.append(&heading);
        header.append(&chips);

        // ---- media tile ----------------------------------------------------
        // The cover. An image with a pixel size, not a picture: a picture never
        // shrinks below the artwork's own size, so one 500x500 cover would size
        // the whole panel. The stack is what swaps it for the stand-in when a
        // player reports no artwork, or artwork that cannot be read.
        let cover = gtk::Image::new();
        cover.add_css_class("cover");
        cover.set_pixel_size(ART - 8);
        // Together with the CSS border-radius this is what keeps the cover inside
        // its rounded tile.
        cover.set_overflow(gtk::Overflow::Hidden);
        let art_fallback = icons::lucide(names::MUSIC, ART_ICON);
        art_fallback.add_css_class("art-fallback");
        let art = gtk::Stack::new();
        art.add_css_class("art");
        art.set_size_request(ART, ART);
        art.set_valign(gtk::Align::Center);
        art.add_named(&cover, Some("cover"));
        art.add_named(&art_fallback, Some("fallback"));
        art.set_visible_child_name("fallback");

        let title = gtk::Label::new(Some("Nothing playing"));
        title.add_css_class("track");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("artist");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
        words.add_css_class("words");
        words.set_valign(gtk::Align::Center);
        words.set_hexpand(true);
        words.append(&title);
        words.append(&subtitle);
        // Capped here rather than in the render: an ellipsised label still asks
        // for its whole text when it is measured, and one long track name would
        // widen the column - and with it the card, whose width is what the
        // panel's hover zone is measured against.
        let words_width = settings.left_width - 2 * TILE_PAD_X - ART - ART_GAP;
        text::cap_width(&title, words_width);
        text::cap_width(&subtitle, words_width);

        let art_row = gtk::Box::new(gtk::Orientation::Horizontal, ART_GAP);
        art_row.add_css_class("art-row");
        art_row.append(&art);
        art_row.append(&words);

        let progress = gtk::ProgressBar::new();
        progress.set_show_text(false);
        progress.set_valign(gtk::Align::Center);
        let elapsed = gtk::Label::new(Some("0:00"));
        elapsed.add_css_class("time");
        elapsed.set_xalign(0.0);
        let total = gtk::Label::new(Some("0:00"));
        total.add_css_class("time");
        total.set_xalign(1.0);
        let times = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        times.append(&elapsed);
        times.append(&total);
        // The bar and the times are one target, not two: three pixels of bar is
        // not something to aim at, so a click anywhere in the block seeks to where
        // it landed.
        let seek = gtk::Box::new(gtk::Orientation::Vertical, 2);
        seek.add_css_class("seek");
        seek.set_tooltip_text(Some("Click to seek"));
        seek.append(&progress);
        seek.append(&times);
        {
            let target = target.clone();
            let playback = playback.clone();
            // The width the fraction is measured against is the widget's own, read
            // when the click lands - the tile can have been re-laid out since.
            let gesture = gtk::GestureClick::new();
            gesture.connect_released(move |gesture, _, x, _| {
                let (Some(widget), Some(current)) = (gesture.widget(), playback.borrow().clone())
                else {
                    return;
                };
                let width = f64::from(widget.width());
                if width <= 0.0 || current.length == 0 {
                    return;
                }
                let seconds = current.second_at((x / width).clamp(0.0, 1.0));
                player_command(mpris::seek(&target.borrow(), seconds));
                // Paint where the track now is rather than where it was: waiting
                // for the next poll would show the old position for as long as the
                // poll takes. The poll reads the truth back anyway, so a seek the
                // player refuses corrects itself on the next tick.
                if let Some(current) = playback.borrow_mut().as_mut() {
                    current.position = seconds * 1_000_000;
                }
            });
            seek.add_controller(gesture);
        }

        // The transport: what is on both sides of the play button is the state of
        // the player rather than another action, and both read their state from
        // what is on screen - which the poll then reads back from the player.
        let shuffle = transport_button(names::SHUFFLE, "Shuffle is off", {
            let target = target.clone();
            let playback = playback.clone();
            move || {
                let on = playback.borrow().as_ref().is_some_and(|it| it.shuffle);
                player_command(mpris::set_shuffle(&target.borrow(), !on));
            }
        });
        let previous = transport_button(names::SKIP_BACK, "Previous track", {
            let target = target.clone();
            move || player_command(mpris::skip(Direction::Previous, &target.borrow()))
        });
        let play = transport_button(names::PLAY, "Play or pause", {
            let target = target.clone();
            move || player_command(mpris::play_pause(&target.borrow()))
        });
        let next = transport_button(names::SKIP_FORWARD, "Next track", {
            let target = target.clone();
            move || player_command(mpris::skip(Direction::Next, &target.borrow()))
        });
        let repeat = transport_button(names::REPEAT, "Repeat is off", {
            let target = target.clone();
            let playback = playback.clone();
            move || {
                let next = playback
                    .borrow()
                    .as_ref()
                    .map(|it| it.repeat.next())
                    .unwrap_or_default();
                player_command(mpris::set_repeat(&target.borrow(), next));
            }
        });
        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        transport.add_css_class("transport");
        transport.set_halign(gtk::Align::Center);
        for button in [&shuffle, &previous, &play, &next, &repeat] {
            transport.append(button);
        }

        // The player's own volume. `change-value` is the *user's* intent - a drag,
        // a scroll, an arrow key - and is not emitted for a programmatic
        // `set_value`, which is what lets the poll paint a level without
        // commanding the player.
        let volume_icon = icons::lucide(names::VOLUME_HIGH, BUTTON_ICON);
        volume_icon.add_css_class("volume-icon");
        volume_icon.set_valign(gtk::Align::Center);
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.add_css_class("volume");
        volume.set_hexpand(true);
        volume.set_valign(gtk::Align::Center);
        volume.set_draw_value(false);
        let volume_value = gtk::Label::new(None);
        volume_value.add_css_class("value");
        volume_value.set_valign(gtk::Align::Center);
        volume_value.set_xalign(1.0);
        volume_value.set_width_chars(4);
        let volume_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        volume_row.add_css_class("volume-row");
        volume_row.append(&volume_icon);
        volume_row.append(&volume);
        volume_row.append(&volume_value);
        {
            let target = target.clone();
            let settle = settle.clone();
            let settling = settling.clone();
            let level = Rc::new(Cell::new(0.0_f64));
            volume.connect_change_value(move |_, _, value| {
                level.set(value);
                settling.set(true);
                let target = target.clone();
                let settling = settling.clone();
                let level = level.clone();
                settle.arm(VOLUME_SETTLE, move || {
                    player_command(mpris::set_volume(&target.borrow(), level.get()));
                    settling.set(false);
                });
                glib::Propagation::Proceed
            });
        }
        {
            // The number follows the fill while a drag is in flight, which the
            // poll cannot do: it is not allowed to touch the slider then.
            let value = volume_value.clone();
            volume.connect_value_changed(move |scale| {
                value.set_text(&format!("{}%", (scale.value() * 100.0).round() as i32));
            });
        }

        let players_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        players_row.add_css_class("players");

        let media = tile("now playing", names::MUSIC);
        media.append(&art_row);
        media.append(&seek);
        media.append(&transport);
        media.append(&volume_row);
        media.append(&players_row);

        // ---- notification tile ---------------------------------------------
        let notif_count = gtk::Label::new(Some("All caught up"));
        notif_count.add_css_class("notif-count");
        notif_count.set_xalign(0.0);

        // The rows. Nothing is put in here at build time: the first line of the
        // feed fills them in, and until then the count above says it all.
        let notif_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        notif_rows.add_css_class("notif-rows");

        let dnd = chip_button(names::BELL, "DND off", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleDnd)
        });
        let open = chip_button(names::LIST, "Open centre", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleCentre)
        });
        let hide = chip_button(names::EYE_OFF, "Hide all", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::HideShown)
        });
        let clear = chip_button(names::TRASH, "Clear all", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::DismissAll)
        });
        clear.add_css_class("danger");

        // Two rows of two: four buttons in a row would set the panel's width, and
        // the panel's width is the calendar's business.
        let notif_actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
        notif_actions.add_css_class("notif-actions");
        for row in [[&dnd, &open], [&hide, &clear]] {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for button in row {
                line.append(button);
            }
            notif_actions.append(&line);
        }

        let notifications = tile("notifications", names::BELL);
        notifications.append(&notif_count);
        notifications.append(&notif_rows);
        notifications.append(&notif_actions);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.add_css_class("column");
        left.set_size_request(settings.left_width, -1);
        left.append(&media);
        left.append(&notifications);

        // ---- calendar tile -------------------------------------------------
        let calendar_title = gtk::Label::new(None);
        calendar_title.add_css_class("cal-title");
        let previous_month = icon_button(names::CHEVRON_LEFT, "Previous month");
        let next_month = icon_button(names::CHEVRON_RIGHT, "Next month");
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        title_row.add_css_class("cal-head-row");
        title_row.append(&previous_month);
        title_row.append(&calendar_title);
        title_row.append(&next_month);

        let calendar_grid = gtk::Grid::new();
        calendar_grid.add_css_class("cal-grid");
        calendar_grid.set_row_spacing(2);
        calendar_grid.set_column_spacing(2);
        calendar_grid.set_halign(gtk::Align::Center);

        let calendar = tile("calendar", names::CLOCK);
        calendar.append(&title_row);
        calendar.append(&calendar_grid);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("column");
        right.set_size_request(settings.right_width, -1);
        right.append(&calendar);

        // ---- the panel -----------------------------------------------------
        let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        handle.add_css_class("handle");
        handle.set_halign(gtk::Align::Center);
        handle.set_size_request(46, 4);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        body.add_css_class("body");
        body.append(&left);
        body.append(&right);

        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sep");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("island");
        root.append(&handle);
        root.append(&header);
        root.append(&separator);
        root.append(&body);

        let view = Rc::new(IslandView {
            root,
            clock,
            date,
            chips,
            art,
            cover,
            title,
            subtitle,
            progress,
            elapsed,
            total,
            seek,
            play,
            shuffle,
            repeat,
            transport,
            volume_row,
            volume,
            volume_value,
            players_row,
            playback,
            target,
            players: RefCell::new(Vec::new()),
            art_url: RefCell::new(String::new()),
            settling,
            notif_count,
            notif_rows,
            ages: RefCell::new(Vec::new()),
            rows: settings.notification_rows,
            column_width: settings.left_width,
            notif_actions,
            dnd,
            hide,
            clear,
            calendar_title,
            calendar_grid,
            month: RefCell::new(Month::today()),
        });

        // The two calendar buttons are wired last, once the view exists: they
        // redraw the very grid they sit above.
        {
            let view = view.clone();
            previous_month.connect_clicked(move |_| view.shift_month(-1));
        }
        {
            let view = view.clone();
            next_month.connect_clicked(move |_| view.shift_month(1));
        }
        view.render_calendar();
        view
    }

    /// The clock and the date. Called once a second while the panel is up.
    pub fn render_clock(&self) {
        let now = glib::DateTime::now_local().ok();
        let text = |pattern: &str| {
            now.as_ref()
                .and_then(|now| now.format(pattern).ok())
                .map(|text| text.to_string())
                .unwrap_or_default()
        };
        self.clock.set_text(&text("%H:%M"));
        self.date.set_text(&text("%A, %d %B %Y"));
    }

    // ---- the media tile ----------------------------------------------------

    /// Read the player and paint the tile.
    ///
    /// This is the one place `main` does not decide *when* to read, and for a
    /// reason: two of the tile's controls change what it shows from the inside - a
    /// click on the progress bar and a click on a player chip - so the tile reads
    /// its own state. `main`'s heartbeat calls this for everything else: every
    /// `media_every_ms`, and once before the panel appears.
    ///
    /// `read_players` also re-reads the list of players, which `main` asks for on
    /// its slower clock: listing them forks `playerctl` a second time, and the list
    /// only changes when a player comes or goes.
    pub fn refresh_media(self: &Rc<Self>, read_players: bool) {
        if read_players {
            *self.players.borrow_mut() = mpris::players();
        }
        let playback = mpris::playback(&self.target.borrow());
        self.render_media(playback.as_ref());
        // After the tile, not before: the chips mark the player that is *on
        // screen*, and that is what the tile has just stored.
        self.render_players();
    }

    /// What is on the tile: the snapshot [`IslandView::refresh_media`] last read.
    /// The header chips and the `status` verb are painted from this, so neither
    /// can disagree with the tile about what is playing.
    pub fn playback(&self) -> Option<Playback> {
        self.playback.borrow().clone()
    }

    /// The player the tile is following, and the players it could follow instead -
    /// what the `status` verb prints.
    pub fn target(&self) -> Player {
        self.target.borrow().clone()
    }

    pub fn players_list(&self) -> Vec<String> {
        self.players.borrow().clone()
    }

    /// What is playing, and everything that follows from it: the cover, the two
    /// lines of text beside it, the progress bar, the five transport buttons, the
    /// player's own volume, and the chips that say which player this is.
    pub fn render_media(self: &Rc<Self>, playback: Option<&Playback>) {
        *self.playback.borrow_mut() = playback.cloned();

        let Some(playback) = playback else {
            self.title.set_text("Nothing playing");
            self.subtitle.set_visible(false);
            self.seek.set_visible(false);
            self.transport.set_sensitive(false);
            self.volume_row.set_visible(false);
            set_button_icon(&self.play, names::PLAY);
            // Nothing loaded: the cover goes back to the stand-in, exactly as it
            // does for artwork that cannot be read.
            self.render_cover("");
            return;
        };

        self.title.set_text(&playback.track.title);
        let subtitle = playback.track.subtitle();
        self.subtitle.set_text(&subtitle);
        self.subtitle.set_visible(!subtitle.is_empty());
        self.render_cover(&playback.track.art);

        // A stream with no length has no progress to draw; a bar stuck at zero
        // would claim it had just started - and there would be nothing to seek in
        // either.
        let known_length = playback.length > 0;
        self.seek.set_visible(known_length);
        if known_length {
            self.progress.set_fraction(playback.fraction());
            self.elapsed.set_text(&playback.elapsed());
            self.total.set_text(&playback.total());
        }

        self.transport.set_sensitive(true);
        set_button_icon(
            &self.play,
            if playback.playing {
                names::PAUSE
            } else {
                names::PLAY
            },
        );

        // The two toggles wear the accent while they are on, the way the DND
        // switch does - a state worth seeing without reading a tooltip. Which
        // *kind* of repeat it is has no drawing of its own, so it goes in the
        // words, which is also the only place a tooltip can say it.
        set_class(&self.shuffle, "on", playback.shuffle);
        set_class(&self.repeat, "on", playback.repeat != Repeat::Off);
        self.shuffle.set_tooltip_text(Some(if playback.shuffle {
            "Shuffle is on"
        } else {
            "Shuffle is off"
        }));
        self.repeat.set_tooltip_text(Some(match playback.repeat {
            Repeat::Off => "Repeat is off",
            Repeat::Track => "Repeating this track",
            Repeat::All => "Repeating the queue",
        }));

        match playback.volume {
            // While a drag is in flight the slider belongs to the pointer: the
            // poll may not pull it back under the finger, and the level the drag
            // settles on is what reaches the player a moment later.
            Some(_) if self.settling.get() => self.volume_row.set_visible(true),
            // A player that reports no volume of its own - a browser usually does
            // not - gets no row at all, rather than one that cannot do anything.
            Some(level) => {
                self.volume.set_value(level);
                self.volume_value
                    .set_text(&format!("{}%", (level * 100.0).round() as i32));
                self.volume_row.set_visible(true);
            }
            None => self.volume_row.set_visible(false),
        }
    }

    /// The cover: the previous one stays until the new one is readable, and the
    /// stand-in shows while the player reports no artwork - or artwork this cannot
    /// read (a `blob:` URL, a picture that fails to decode).
    fn render_cover(self: &Rc<Self>, url: &str) {
        if *self.art_url.borrow() == url {
            return;
        }
        *self.art_url.borrow_mut() = url.to_owned();
        // A new track starts on the stand-in rather than on the last track's
        // cover, which would misrepresent what is playing.
        self.art.set_visible_child_name("fallback");
        if url.is_empty() {
            return;
        }

        let me = Rc::downgrade(self);
        let expected = url.to_owned();
        art::load(url, move |texture| {
            let Some(me) = me.upgrade() else {
                return;
            };
            // The cover only lands if it is still the track on the tile.
            if *me.art_url.borrow() != expected {
                return;
            }
            match texture {
                Some(texture) => {
                    me.cover.set_paintable(Some(&texture));
                    me.art.set_visible_child_name("cover");
                }
                None => me.art.set_visible_child_name("fallback"),
            }
        });
    }

    /// The chips that choose which player the tile follows, and the healing that
    /// goes with them: a player this tile was switched to can go away (a browser
    /// tab closing), and then it is back to whichever playerctl picks.
    ///
    /// The row is hidden unless there is a choice to make - a list of one is not a
    /// choice, it is a label pretending to be a button.
    fn render_players(self: &Rc<Self>) {
        let players = self.players.borrow().clone();

        // A player that has gone cannot stay chosen. An *empty* list is not a
        // judgement about the chosen one - it is what a failed listing looks like -
        // so nothing is reset then.
        let gone = match self.target.borrow().name() {
            Some(name) => !players.is_empty() && !players.iter().any(|player| player == name),
            None => false,
        };
        if gone {
            *self.target.borrow_mut() = Player::Active;
        }
        // Which chip the tile is showing: a chosen player by name, or - while it is
        // following playerctl's own choice - the one the track on screen came from.
        // `{{playerName}}` is the plain name, which is exactly what a chip is
        // labelled with, so the two match without a lookup.
        let chosen = match self.target.borrow().name() {
            Some(name) => Some(name.to_owned()),
            None => {
                let playback = self.playback.borrow();
                let player = playback
                    .as_ref()
                    .map(|it| it.track.player.clone())
                    .unwrap_or_default();
                (!player.is_empty()).then_some(player)
            }
        };

        while let Some(child) = self.players_row.first_child() {
            self.players_row.remove(&child);
        }
        self.players_row.set_visible(players.len() > 1);
        for name in players {
            let label = gtk::Label::new(Some(&name));
            label.add_css_class("player-name");
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text::cap_width(&label, PLAYER_CHIP);
            let chip = gtk::Button::new();
            chip.add_css_class("player-chip");
            chip.set_child(Some(&label));
            chip.set_tooltip_text(Some(&format!("Follow {name}")));
            chip.set_focus_on_click(false);
            chip.set_can_focus(false);
            set_class(&chip, "selected", chosen.as_deref() == Some(name.as_str()));

            let weak = Rc::downgrade(self);
            let target = self.target.clone();
            chip.connect_clicked(move |_| {
                *target.borrow_mut() = Player::named(name.clone());
                // Read straight away rather than waiting for the heartbeat: the
                // whole point of the chip is that the *other* player is on the
                // tile now, and a poll's worth of the old track would be a lie.
                // Only the accent moves by hand, and only the tile is painted -
                // rebuilding the row this button belongs to, from inside its own
                // click handler, is a risk with nothing to gain: the next
                // heartbeat paints it from the truth anyway.
                if let Some(view) = weak.upgrade() {
                    view.select_player(&name);
                    let playback = mpris::playback(&target.borrow());
                    view.render_media(playback.as_ref());
                }
            });
            self.players_row.append(&chip);
        }
    }

    /// Move the accent to the chip that was clicked, without rebuilding the row.
    fn select_player(&self, chosen: &str) {
        let mut child = self.players_row.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Some(button) = widget.downcast_ref::<gtk::Button>() else {
                continue;
            };
            let label = button.child().and_downcast::<gtk::Label>();
            let mine = label.is_some_and(|label| label.text() == chosen);
            set_class(button, "selected", mine);
        }
    }

    /// The notification tile: the count, the rows, and the buttons that only make
    /// sense when there is something to act on.
    pub fn render_notifications(&self, notifications: &Notifications, history: &History) {
        let count = notifications.count;
        self.notif_count.set_text(&match count {
            0 => "All caught up".to_string(),
            1 => "1 unread notification".to_string(),
            count => format!("{count} unread notifications"),
        });
        // Nothing waiting is good news, and good news need not be the brightest
        // thing in the tile.
        set_class(&self.notif_count, "quiet", count == 0);

        label_of(&self.dnd).set_text(if notifications.dnd {
            "DND on"
        } else {
            "DND off"
        });
        set_chip_icon(
            &self.dnd,
            if notifications.dnd {
                names::BELL_OFF
            } else {
                names::BELL
            },
        );
        set_class(&self.dnd, "on", notifications.dnd);

        // Hiding or clearing nothing is not a thing to offer.
        let has_notifications = count > 0;
        self.hide.set_sensitive(has_notifications);
        self.clear.set_sensitive(has_notifications);
        self.notif_actions.set_visible(true);

        self.render_rows(history, notifications);
    }

    /// The rows: the newest few notifications the bus carried, made again for a
    /// new list.
    ///
    /// Made again rather than reconciled, because the list changes when a
    /// notification arrives - a handful of times an hour - and working out which
    /// row moved would be more code than building them over. The ages are the
    /// part that changes *without* a new list, and they are kept in `ages` for
    /// [`IslandView::render_ages`] to touch while the panel is up.
    fn render_rows(&self, history: &History, notifications: &Notifications) {
        while let Some(child) = self.notif_rows.first_child() {
            self.notif_rows.remove(&child);
        }
        self.ages.borrow_mut().clear();

        let now = Instant::now();
        let shown: Vec<&Item> = history.items().iter().take(self.rows).collect();
        for (position, item) in shown.iter().enumerate() {
            if position > 0 {
                // A hairline between two notifications rather than a box around
                // each: the tile is a surface already, and a box inside a box
                // inside a box is where a panel starts to look noisy.
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("notif-sep");
                self.notif_rows.append(&separator);
            }
            let (row, age) = notification_row(item, self.text_width(), now);
            self.notif_rows.append(&row);
            self.ages.borrow_mut().push((age, (*item).clone()));
        }

        // What the tile cannot show, admitted rather than hidden: the count is the
        // daemon's, and the rows are the ones that arrived while this process was
        // watching. With no row on screen at all - a panel that has just started,
        // or a bus that refuses to be monitored - there is nothing to count
        // against, so the line above stands alone.
        let hidden = usize::try_from(notifications.count)
            .unwrap_or(0)
            .saturating_sub(shown.len());
        if hidden > 0 && !shown.is_empty() {
            let more = gtk::Label::new(Some(&format!("+{hidden} more")));
            more.add_css_class("notif-more");
            more.set_xalign(0.0);
            self.notif_rows.append(&more);
        }
    }

    /// Keep the ages honest: a row that says `5 min` has to become `6 min` with
    /// nothing having arrived in between. Called on the heartbeat, and only while
    /// the panel is up.
    pub fn render_ages(&self) {
        let now = Instant::now();
        for (label, item) in self.ages.borrow().iter() {
            label.set_text(&item.age(now));
        }
    }

    /// How wide a row's text column is: the panel's left column, less the padding
    /// the tile and the row put around it, less the icon badge and the gap beside
    /// it.
    fn text_width(&self) -> i32 {
        (self.column_width - 2 * TILE_PAD_X - 2 * ROW_PAD_X - ROW_ART).max(40)
    }

    /// The state chips in the header: what is playing, what is waiting, and
    /// whether the control centre is up.
    pub fn render_chips(&self, playback: Option<&Playback>, notifications: &Notifications) {
        let mut wanted: Vec<(String, &str)> = Vec::new();
        if let Some(playback) = playback {
            wanted.push((
                if playback.playing {
                    "playing".to_string()
                } else {
                    "paused".to_string()
                },
                if playback.playing { "playing" } else { "dim" },
            ));
        }
        if notifications.count > 0 {
            wanted.push((format!("{} unread", notifications.count), "warn"));
        }
        if notifications.dnd {
            wanted.push(("DND".to_string(), "warn"));
        } else if notifications.inhibited {
            wanted.push(("inhibited".to_string(), "warn"));
        }
        if notifications.centre_open {
            wanted.push(("centre open".to_string(), "accent"));
        }

        // Rebuilding four labels on every repaint is cheaper than working out
        // which of them changed, and the repaints are once a second.
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for (text, class) in wanted {
            let chip = gtk::Label::new(Some(&text));
            chip.add_css_class("chip");
            chip.add_css_class(class);
            self.chips.append(&chip);
        }
    }

    /// Page the calendar by whole months.
    pub fn shift_month(&self, delta: i32) {
        let next = self.month.borrow().shift(delta);
        *self.month.borrow_mut() = next;
        self.render_calendar();
    }

    /// Draw the month grid.
    pub fn render_calendar(&self) {
        let month = *self.month.borrow();
        self.calendar_title.set_text(&month.title());

        while let Some(child) = self.calendar_grid.first_child() {
            self.calendar_grid.remove(&child);
        }

        let today = Month::today();
        let now = glib::DateTime::now_local().ok();
        let today_day = now.as_ref().map(|now| now.day_of_month()).unwrap_or(0);

        // The weekday headings, then the six weeks of the month.
        for (column, name) in WEEKDAYS.iter().enumerate() {
            let label = gtk::Label::new(Some(name));
            label.add_css_class("cal-head");
            self.calendar_grid.attach(&label, column as i32, 0, 1, 1);
        }
        for (row, week) in month.weeks().iter().enumerate() {
            for (column, day) in week.iter().enumerate() {
                let label = gtk::Label::new(None);
                label.add_css_class("cal-day");
                if *day > 0 {
                    label.set_text(&day.to_string());
                }
                // Saturday and Sunday sit at the right of a Monday-first week.
                if column >= 5 {
                    label.add_css_class("weekend");
                }
                if *day > 0 && *day == today_day && month == today {
                    label.add_css_class("today");
                }
                self.calendar_grid
                    .attach(&label, column as i32, row as i32 + 1, 1, 1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small building blocks
// ---------------------------------------------------------------------------

/// A tile: the nested surface inside the card, with its section heading.
fn tile(title: &str, icon: &str) -> gtk::Box {
    let heading = icons::lucide(icon, SMALL_ICON);
    heading.add_css_class("section-icon");
    let name = gtk::Label::new(Some(title));
    name.add_css_class("section");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("section-row");
    line.append(&heading);
    line.append(&name);

    let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tile.add_css_class("tile");
    tile.append(&line);
    tile
}

/// One notification row: the application's badge, what it said, and how long ago.
///
/// The age label comes back with the row because it is the one part of a row that
/// changes while nothing else does - see [`IslandView::render_ages`].
fn notification_row(item: &Item, text_width: i32, now: Instant) -> (gtk::Box, gtk::Label) {
    // The badge the icon sits on. It is drawn whether or not there is a picture,
    // so every row has the same shape: the icon theme's answer decides what sits
    // inside it, not the box.
    const ART: i32 = 24;
    let art = gtk::Stack::new();
    art.add_css_class("notif-art");
    art.set_size_request(ART, ART);
    art.set_halign(gtk::Align::Start);
    art.set_valign(gtk::Align::Center);
    let image = gtk::Image::new();
    image.set_pixel_size(ART - 8);
    let letter = gtk::Label::new(Some(&text::initial(&item.app)));
    letter.add_css_class("notif-letter");
    art.add_named(&image, Some("icon"));
    art.add_named(&letter, Some("letter"));
    art.set_visible_child_name("letter");
    // The hint is an icon name or the path of a file the sender attached, and
    // `by_name` answers both. With no hint at all the application's own name is
    // the best guess - and that one goes through the desktop entries, because a
    // name is not an icon name until it has been looked up (`Telegram` is the
    // application, `telegram` the icon). An application the theme has nothing for
    // stands on its first letter, which still says which one it is.
    let paintable = if item.icon.is_empty() {
        icons::paintable(&item.app, ART - 8)
    } else {
        icons::by_name(&item.icon, ART - 8)
    };
    if let Some(paintable) = paintable {
        image.set_paintable(Some(&paintable));
        art.set_visible_child_name("icon");
    }

    // The application and the age on one line, with the age pushed to the end.
    // A sender that names no application gets its summary up there instead - and
    // then there is nothing left for a second line.
    let named = !item.app.is_empty();
    let app = gtk::Label::new(Some(if named { &item.app } else { &item.title }));
    app.add_css_class("notif-app");
    app.set_xalign(0.0);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    app.set_hexpand(true);
    let age = gtk::Label::new(Some(&item.age(now)));
    age.add_css_class("notif-age");
    age.set_xalign(1.0);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&app);
    head.append(&age);

    let title = gtk::Label::new(Some(&item.title));
    title.add_css_class("notif-text");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let words = gtk::Box::new(gtk::Orientation::Vertical, 1);
    words.set_hexpand(true);
    words.append(&head);
    if named {
        words.append(&title);
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("notif-row");
    if item.urgency >= 2 {
        // What swaync holds on screen until it is dealt with, so it is worth
        // noticing at a glance in the list as well.
        row.add_css_class("critical");
    }
    // What the sender wrote, in full: the row has room for one line of it.
    row.set_tooltip_text(Some(&item.tooltip()));
    row.append(&art);
    row.append(&words);

    // Capped, both of them: an ellipsised label still asks for its whole text when
    // it is measured, and a row that asked for too much would widen the column -
    // and with it the card, whose width is what the panel's hover zone is
    // measured against.
    text::cap_width(&app, text_width - ROW_AGE - 6);
    text::cap_width(&title, text_width);

    (row, age)
}

/// A button with an icon and a word, the way every control in this panel is
/// built: the icon carries the meaning, the word removes the doubt.
fn chip_button(icon: &str, label: &str, action: impl Fn() + 'static) -> gtk::Button {
    let icon = icons::lucide(icon, SMALL_ICON);
    let text = gtk::Label::new(Some(label));
    text.add_css_class("chip-text");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&icon);
    row.append(&text);

    let button = gtk::Button::new();
    button.add_css_class("chipbtn");
    button.set_child(Some(&row));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.connect_clicked(move |_| action());
    button
}

/// A button that is only an icon, for the transport and the calendar paging.
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button
}

fn transport_button(icon: &str, tooltip: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = icon_button(icon, tooltip);
    button.add_css_class("transport-button");
    button.connect_clicked(move |_| action());
    button
}

/// The text label inside a [`chip_button`] - the half that changes wording while
/// the icon stays.
fn label_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.last_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip button's second child is its label")
}

/// The icon inside a [`chip_button`] - the half that stays while the word
/// beside it changes.
fn icon_of(button: &gtk::Button) -> gtk::Image {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Image>()
        .expect("a chip button's first child is its icon")
}

/// Re-point a chip's icon at another drawing, which is how the DND switch says
/// which way it is switched without the button being rebuilt.
fn set_chip_icon(button: &gtk::Button, icon: &str) {
    icons::set_lucide(&icon_of(button), icon, SMALL_ICON);
}

/// The same for a button that is *only* an icon (see [`icon_button`]): the play
/// button becomes a pause in place, because rebuilding it would take the button
/// out from under the pointer that is resting on it.
fn set_button_icon(button: &gtk::Button, icon: &str) {
    if let Some(image) = button.child().and_downcast::<gtk::Image>() {
        icons::set_lucide(&image, icon, BUTTON_ICON);
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// Run one of the player's commands. Fire and forget, like the notification
/// buttons: what the player did comes back through the next poll, so the tile
/// never guesses - and a player that refuses a command (a browser asked to
/// shuffle, a stream with no length to seek in) is worth a line on stderr, not a
/// dialog inside a popup.
fn player_command(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("hypr-osd-island: {error}");
    }
}

/// Run one of the daemon's commands. Fire and forget: what changes comes back
/// through the notification feed, so the panel never guesses at the result.
fn notify_command(program: &str, command: notify::Command) {
    let _ = std::process::Command::new(program)
        .args(command.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
