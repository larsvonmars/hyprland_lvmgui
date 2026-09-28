//! A contact sheet of every bundled Lucide icon, for looking at.
//!
//! Not part of any element: this is how the icon set is checked on a real
//! screen - that the strokes are not too heavy at the size a bar pill draws
//! them (14px next to 13px text), that GTK really does paint them in the
//! widget's CSS colour, and that none of them has come out as "image missing".
//! It is also how the one measurement in this change was made: Lucide's
//! `signal-*` icons put their ink in the lower left of the box, which at pill
//! size is a dot in a corner, and the wifi arcs replaced them because of what
//! this sheet showed.
//!
//! ```sh
//! cargo run -p hypr-osd-core --example icon-sheet
//! grim -T "$(hyprctl -j clients | python3 -c 'import json,sys; print([c["stableId"] for c in json.load(sys.stdin) if c["class"] == "com.schells2.osd.icon-sheet"][0])')" /tmp/icon-sheet.png
//! ```

use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};

const SHEET: &str = "
.sheet { background-color: #0b0f15; }
.label { color: #4d5661; font-family: monospace; font-size: 9px; }
.fg { color: #dde7ef; }
.dim { color: #8b98a8; }
.accent { color: #33ccff; }
.accent2 { color: #00ff99; }
.warn { color: #ffcc66; }
.crit { color: #ff5566; }
.text { color: #dde7ef; font-family: monospace; font-size: 13px; }
";

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("com.schells2.osd.icon-sheet")
        .build();
    app.connect_activate(|app| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(SHEET);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("a display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let column = gtk::Box::new(gtk::Orientation::Vertical, 14);
        column.add_css_class("sheet");
        column.set_margin_top(16);
        column.set_margin_bottom(16);
        column.set_margin_start(16);
        column.set_margin_end(16);

        // Every icon at a size the drawings can be judged at, with its name. The
        // elements draw them at 10-16px, but a contact sheet is for *looking*:
        // GTK's SVG engine is the thing being checked here, and a silhouette is
        // much easier to spot at 26px than at 14.
        column.append(&captioned("every icon, 26px, @fg", names::ALL, "fg"));
        // The same drawing at every size an element asks for.
        column.append(&ladder());
        // The same drawing in every colour an element has to say something in.
        column.append(&colours());
        // And one in a sentence, because that is where it has to sit.
        column.append(&in_context());
        // The two ladders a pill swaps between as a reading changes.
        column.append(&ladders());

        let window = gtk::ApplicationWindow::new(app);
        window.add_css_class("sheet");
        window.set_child(Some(&column));
        window.present();
    });
    app.run()
}

fn icon(name: &str, size: i32, class: &str) -> gtk::Image {
    let image = icons::lucide(name, size);
    image.add_css_class(class);
    image
}

fn captioned(title: &str, sheet: &[(&str, &str)], class: &str) -> gtk::Box {
    let head = gtk::Label::new(Some(title));
    head.add_css_class("label");
    head.set_xalign(0.0);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(10);
    for (index, (constant, name)) in sheet.iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        cell.append(&icon(name, 26, class));
        let caption = gtk::Label::new(Some(name));
        caption.add_css_class("label");
        cell.append(&caption);
        grid.attach(&cell, (index % 12) as i32, (index / 12) as i32, 1, 1);
        let _ = constant;
    }

    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.append(&head);
    boxed.append(&grid);
    boxed
}

fn ladder() -> gtk::Box {
    let head = gtk::Label::new(Some("one icon, every size an element asks for (12-28)"));
    head.add_css_class("label");
    head.set_xalign(0.0);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_valign(gtk::Align::Center);
    for size in [12, 13, 14, 15, 16, 18, 20, 22, 28] {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        cell.append(&icon(names::VOLUME_HIGH, size, "fg"));
        let caption = gtk::Label::new(Some(&size.to_string()));
        caption.add_css_class("label");
        cell.append(&caption);
        row.append(&cell);
    }
    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.append(&head);
    boxed.append(&row);
    boxed
}

fn colours() -> gtk::Box {
    let head = gtk::Label::new(Some("one icon, every colour the theme speaks in"));
    head.add_css_class("label");
    head.set_xalign(0.0);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_valign(gtk::Align::Center);
    for class in ["fg", "dim", "accent", "accent2", "warn", "crit"] {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        cell.append(&icon(names::VOLUME_HIGH, 16, class));
        cell.append(&icon(names::BATTERY_CHARGING, 16, class));
        cell.append(&icon(names::WIFI_OFF, 16, class));
        let caption = gtk::Label::new(Some(class));
        caption.add_css_class("label");
        cell.append(&caption);
        row.append(&cell);
    }
    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.append(&head);
    boxed.append(&row);
    boxed
}

/// An icon inside the sentence it will sit in, which is the only way to judge
/// whether the strokes are too heavy for 13px text.
fn in_context() -> gtk::Box {
    let head = gtk::Label::new(Some("in the pill it belongs to"));
    head.add_css_class("label");
    head.set_xalign(0.0);

    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.append(&head);
    for (icon, text) in [
        (names::VOLUME_LOW, "72%"),
        (names::BATTERY_MEDIUM, "58%"),
        (names::WIFI_HIGH, "83%"),
        (names::MEMORY, "41%"),
        (names::TEMPERATURE, "59°"),
        (names::CPU, "12%"),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&icon_widget(icon));
        let label = gtk::Label::new(Some(text));
        label.add_css_class("text");
        row.append(&label);
        boxed.append(&row);
    }
    boxed
}

fn icon_widget(name: &str) -> gtk::Image {
    icon(name, 14, "fg")
}

/// The ladders: one icon per step, at the size a pill draws them, with the
/// reading that chooses the step.
fn ladders() -> gtk::Box {
    let head = gtk::Label::new(Some("the ladders, at pill size"));
    head.add_css_class("label");
    head.set_xalign(0.0);

    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.append(&head);
    for (steps, readings) in [
        (
            [
                names::WIFI_ZERO,
                names::WIFI_LOW,
                names::WIFI_HIGH,
                names::WIFI,
            ],
            ["12%", "38%", "64%", "91%"],
        ),
        (
            [
                names::VOLUME_OFF,
                names::VOLUME_LOW,
                names::VOLUME_HIGH,
                names::VOLUME_HIGH,
            ],
            ["mute", "33%", "66%", "90%"],
        ),
        (
            [
                names::BATTERY_EMPTY,
                names::BATTERY_LOW,
                names::BATTERY_MEDIUM,
                names::BATTERY_FULL,
            ],
            ["8%", "30%", "55%", "90%"],
        ),
        (
            [
                names::WORKSPACE_ACTIVE,
                names::WORKSPACE_IDLE,
                names::WORKSPACE_IDLE,
                names::WORKSPACE_IDLE,
            ],
            ["here", "2", "3", "4"],
        ),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.set_valign(gtk::Align::Center);
        for (icon, reading) in steps.into_iter().zip(readings) {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            cell.append(&icon_widget(icon));
            let label = gtk::Label::new(Some(reading));
            label.add_css_class("text");
            cell.append(&label);
            cell.set_size_request(74, -1);
            row.append(&cell);
        }
        boxed.append(&row);
    }
    boxed
}
