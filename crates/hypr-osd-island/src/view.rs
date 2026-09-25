//! The island panel: what is playing, what is waiting, and what day it is.
//!
//! Three columns of information in one card that unfolds from the bar's clock:
//!
//! ```text
//!  ┌───────────── handle ─────────────┐
//!  │ 16:04                  [now playing] [3 unread] │
//!  │ Friday, 25 September   ─────────────────────────  │
//!  │ ┌─ now playing ─────┐ ┌─ calendar ─────────────┐ │
//!  │ │ title             │ │ ‹ September 2026 ›     │ │
//!  │ │ artist            │ │ Mo Tu We Th Fr Sa Su   │ │
//!  │ │ ▁▁▁▁▁▁ 1:15 4:02  │ │ …                      │ │
//!  │ │ [‹] [▶] [›]       │ │                        │ │
//!  │ └───────────────────┘ └────────────────────────┘ │
//!  │ ┌─ notifications ───┐                            │
//!  │ │ 3 unread          │                            │
//!  │ │ [dnd] [open]      │                            │
//!  │ │ [hide] [clear]    │                            │
//!  │ └───────────────────┘                            │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! The view paints what it is handed and runs the commands its buttons mean. It
//! does not read anything itself: `main` polls MPRIS while the panel is up, the
//! notification feed arrives on its own, and the clock ticks - so there is one
//! place that decides *when* to read, and one place that decides how to draw.
//!
//! The card itself - the fill, the border, the shadow, the 16px radius - is the
//! shell's (`box.card` in `base.css`), because this panel is a card like every
//! OSD in the collection; what is here is only what goes inside one.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::mpris::Playback;

use crate::calendar::Month;
use crate::notify::{self, Notifications};
use crate::Settings;

const CLOCK: &str = "\u{f017}";
const PLAY: &str = "\u{f04b}";
const PAUSE: &str = "\u{f04c}";
const PREVIOUS: &str = "\u{f048}";
const NEXT: &str = "\u{f051}";
const BELL: &str = "\u{f0f3}";
const BELL_OFF: &str = "\u{f1f6}";
const LIST: &str = "\u{f03a}";
const EYE_OFF: &str = "\u{f070}";
const TRASH: &str = "\u{f1f8}";
const CHEVRON_LEFT: &str = "\u{f104}";
const CHEVRON_RIGHT: &str = "\u{f105}";
const MUSIC: &str = "\u{f001}";

/// The weekday headings, Monday first - the same order [`Month::weeks`] lays the
/// grid out in.
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

pub struct IslandView {
    /// The panel's content: the card's child.
    pub root: gtk::Box,
    clock: gtk::Label,
    date: gtk::Label,
    chips: gtk::Box,
    track: gtk::Label,
    artist: gtk::Label,
    progress: gtk::ProgressBar,
    elapsed: gtk::Label,
    total: gtk::Label,
    play: gtk::Button,
    /// The row holding the progress bar and the times: hidden together, because a
    /// time without a bar says nothing.
    progress_row: gtk::Box,
    transport: gtk::Box,
    notif_count: gtk::Label,
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
        let track = gtk::Label::new(Some("Nothing playing"));
        track.add_css_class("track");
        track.set_xalign(0.0);
        track.set_ellipsize(gtk::pango::EllipsizeMode::End);
        track.set_max_width_chars(28);
        let artist = gtk::Label::new(None);
        artist.add_css_class("artist");
        artist.set_xalign(0.0);
        artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        artist.set_max_width_chars(30);

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
        let progress_row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        progress_row.append(&progress);
        progress_row.append(&times);

        let previous = transport_button(PREVIOUS, "Previous track", || {
            let _ = hypr_osd_core::mpris::skip(hypr_osd_core::mpris::Direction::Previous);
        });
        let play = transport_button(PLAY, "Play or pause", || {
            let _ = hypr_osd_core::mpris::play_pause();
        });
        let next = transport_button(NEXT, "Next track", || {
            let _ = hypr_osd_core::mpris::skip(hypr_osd_core::mpris::Direction::Next);
        });
        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        transport.add_css_class("transport");
        transport.set_halign(gtk::Align::Center);
        for button in [&previous, &play, &next] {
            transport.append(button);
        }

        let media = tile("now playing", MUSIC);
        media.append(&track);
        media.append(&artist);
        media.append(&progress_row);
        media.append(&transport);

        // ---- notification tile ---------------------------------------------
        let notif_count = gtk::Label::new(Some("All caught up"));
        notif_count.add_css_class("notif-count");
        notif_count.set_xalign(0.0);

        let dnd = chip_button(BELL, "DND off", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleDnd)
        });
        let open = chip_button(LIST, "Open centre", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleCentre)
        });
        let hide = chip_button(EYE_OFF, "Hide all", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::HideShown)
        });
        let clear = chip_button(TRASH, "Clear all", {
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

        let notifications = tile("notifications", BELL);
        notifications.append(&notif_count);
        notifications.append(&notif_actions);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.add_css_class("column");
        left.set_size_request(settings.left_width, -1);
        left.append(&media);
        left.append(&notifications);

        // ---- calendar tile -------------------------------------------------
        let calendar_title = gtk::Label::new(None);
        calendar_title.add_css_class("cal-title");
        let previous_month = icon_button(CHEVRON_LEFT, "Previous month");
        let next_month = icon_button(CHEVRON_RIGHT, "Next month");
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

        let calendar = tile("calendar", CLOCK);
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
            track,
            artist,
            progress,
            elapsed,
            total,
            play,
            progress_row,
            transport,
            notif_count,
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

    /// What is playing, or nothing at all.
    pub fn render_media(&self, playback: Option<&Playback>) {
        let Some(playback) = playback else {
            self.track.set_text("Nothing playing");
            self.artist.set_visible(false);
            self.progress_row.set_visible(false);
            self.transport.set_sensitive(false);
            self.play.set_label(PLAY);
            return;
        };

        self.track.set_text(&playback.track.title);
        let subtitle = playback.track.subtitle();
        self.artist.set_text(&subtitle);
        self.artist.set_visible(!subtitle.is_empty());

        // A stream with no length has no progress to draw; a bar stuck at zero
        // would claim it had just started.
        let known_length = playback.length > 0;
        self.progress_row.set_visible(known_length);
        if known_length {
            self.progress.set_fraction(playback.fraction());
            self.elapsed.set_text(&playback.elapsed());
            self.total.set_text(&playback.total());
        }

        self.transport.set_sensitive(true);
        self.play
            .set_label(if playback.playing { PAUSE } else { PLAY });
    }

    /// The notification tile, and the two buttons that only make sense when there
    /// is something to act on.
    pub fn render_notifications(&self, notifications: &Notifications) {
        self.notif_count.set_text(&match notifications.count {
            0 => "All caught up".to_string(),
            1 => "1 unread notification".to_string(),
            count => format!("{count} unread notifications"),
        });

        label_of(&self.dnd).set_text(if notifications.dnd {
            "DND on"
        } else {
            "DND off"
        });
        set_glyph(&self.dnd, if notifications.dnd { BELL_OFF } else { BELL });
        set_class(&self.dnd, "on", notifications.dnd);

        // Hiding or clearing nothing is not a thing to offer.
        let has_notifications = notifications.count > 0;
        self.hide.set_sensitive(has_notifications);
        self.clear.set_sensitive(has_notifications);
        self.notif_actions.set_visible(true);
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
fn tile(title: &str, glyph: &str) -> gtk::Box {
    let heading = gtk::Label::new(Some(glyph));
    heading.add_css_class("section-glyph");
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

/// A button with an icon and a word, the way every control in this panel is
/// built: the icon carries the meaning, the word removes the doubt.
fn chip_button(glyph: &str, label: &str, action: impl Fn() + 'static) -> gtk::Button {
    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("chip-glyph");
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
fn icon_button(glyph: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(glyph);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button
}

fn transport_button(glyph: &str, tooltip: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = icon_button(glyph, tooltip);
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

/// The icon label inside a [`chip_button`].
fn glyph_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip button's first child is its glyph")
}

fn set_glyph(button: &gtk::Button, glyph: &str) {
    let label = glyph_of(button);
    if label.text() != glyph {
        label.set_text(glyph);
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
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
