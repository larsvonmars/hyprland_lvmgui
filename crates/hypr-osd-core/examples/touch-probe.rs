//! Throwaway probe for the touch layer: a card with a tap catcher around it.
//!
//! Not part of the collection - this exists to check the tap catcher's surface
//! on the real compositor (does it map, does it dim the screen, does the hole
//! stay over the card) before every element depends on it.
//!
//!     cargo run -p hypr-osd-core --example touch-probe show
//!     grim /tmp/touch-probe.png
//!     hyprctl layers | grep -A2 hypr-osd

use std::rc::Rc;

use gtk::prelude::*;
use hypr_osd_core::{css, run, Content, Hole, Opts, Osd, Placement};

const APP_ID: &str = "com.schells2.osd.touchprobe";

fn main() {
    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(""),
        namespace: "hypr-osd".to_string(),
        width: 420,
        placement: Placement::Center,
        ..Opts::default()
    };

    let build = Box::new(|_osd: &Rc<Osd>| {
        let label = gtk::Label::new(Some("tap catcher probe"));
        label.set_size_request(420, 220);
        Content::Single(label.upcast())
    });

    let handle = Rc::new(|osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
        match args.first().map(String::as_str) {
            Some("show") => {
                osd.show();
                // The card is 420x220, centred on the focused monitor. Measure
                // the monitor so the hole lands on it rather than on a guess.
                let (mw, mh) = hypr_osd_core::monitors::first()
                    .map(|monitor| {
                        let geometry = monitor.geometry();
                        (geometry.width(), geometry.height())
                    })
                    .unwrap_or((1368, 912));
                let hole = Hole {
                    x: (mw - 420) / 2,
                    y: (mh - 220) / 2,
                    width: 420,
                    height: 220,
                };
                osd.tap_catcher(Some(hole), || eprintln!("hypr-osd: tapped outside"));
                Ok(format!(
                    "showing, hole {hole:?}, touch {}",
                    hypr_osd_core::touch::tablet()
                ))
            }
            Some("hide") => {
                osd.hide();
                osd.tap_catcher(None, || {});
                Ok("hidden".to_string())
            }
            Some("status") => Ok(format!(
                "touch present {}, tablet {}, osk {:?}, keyboard {}",
                hypr_osd_core::touch::present(),
                hypr_osd_core::touch::tablet(),
                hypr_osd_core::touch::osk().program(),
                hypr_osd_core::touch::has_keyboard()
            )),
            _ => Err("usage: touch-probe show|hide|status".to_string()),
        }
    });

    run(opts, build, handle);
}
