// Throwaway prototype: does a GtkFixed child animate smoothly, and does
// `set_overflow(Hidden)` clip a child that is bigger than its parent?
//
//   cargo run -p hypr-osd-core --example island-proto
//
// The window is a slab at the top left of the focused screen. The red shell
// grows from a pill-sized rect to a card-sized one; the blue body inside it is
// the final card and must be clipped to the shell (aligned to its right edge).

use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("com.schells2.osd.island-proto")
        .build();
    app.connect_activate(|app| {
        let css = gtk::CssProvider::new();
        css.load_from_string(
            "window { background: #202020; }
             box.shell { background: #b04040; border-radius: 0 0 22px 22px; }
             box.body { background: #4060b0; }
             label { color: #ffffff; font-size: 20px; }",
        );
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let window = gtk::Window::new();
        window.set_default_size(900, 700);
        window.set_title(Some("island-proto"));

        let canvas = gtk::Fixed::new();
        window.set_child(Some(&canvas));

        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.add_css_class("shell");
        shell.set_overflow(gtk::Overflow::Hidden);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("body");
        body.set_size_request(500, 520);
        body.set_halign(gtk::Align::End);
        body.set_valign(gtk::Align::Start);
        body.set_margin_top(60);
        let label = gtk::Label::new(Some("CARD BODY"));
        body.append(&label);
        shell.append(&body);

        canvas.put(&shell, 300.0, 0.0);
        shell.set_size_request(200, 40);

        window.present();

        let started = Instant::now();
        let duration = Duration::from_millis(1500);
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let t = (started.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
            let ease = 1.0 - (1.0 - t).powi(3);
            let width = 200.0 + (500.0 - 200.0) * ease;
            let height = 40.0 + (600.0 - 40.0) * ease;
            shell.set_size_request(width as i32, height as i32);
            let _ = (&canvas, &window);
            if t >= 1.0 {
                println!("width {width} height {height} done");
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    });
    app.run()
}
