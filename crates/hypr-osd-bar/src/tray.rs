//! The system tray: a StatusNotifierItem host.
//!
//! MPRIS is not the only thing that lives on the session bus. Application
//! indicators (Steam, Discord, nm-applet, the Nextcloud client, `swaync`'s own
//! tray icon …) publish themselves through the StatusNotifierItem protocol, and
//! somebody has to *host* them or they are simply invisible. That somebody is
//! the bar: this module implements both halves of the protocol over the session
//! bus, with the same `gio` the bar uses to talk to Hyprland and MPRIS.
//!
//! The two halves, because the naming is genuinely confusing:
//!
//! * **The watcher** (`org.kde.StatusNotifierWatcher`) is a service an item
//!   registers *with* - "here is my bus name, draw me". There is usually only
//!   one on a session, and on a desktop without one of the small tray daemons it
//!   has to be the bar. Owning this name is what makes the icons appear at all.
//! * **The host** (`org.kde.StatusNotifierHost-<pid>`) is what the watcher tells
//!   items about, so an item can ask whether anybody is listening. We own that
//!   name too and report ourselves as registered.
//!
//! What it does: renders each item's icon (its pixmap if it sent one, otherwise
//! an icon-theme lookup by name), and passes clicks on - `Activate` for a left
//! click, `SecondaryActivate` for a middle one, `ContextMenu` for a right one.
//!
//! What it deliberately does not do: draw the item's *menu*. The
//! StatusNotifierItem protocol points at a second protocol
//! (`com.canonical.dbusmenu`) for that, and rendering a menu tree with its own
//! submenus, toggles and radio groups is a project of its own. Items that
//! implement `ContextMenu` themselves get theirs; the rest do nothing on a right
//! click. The left click - the gesture that matters - always works.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::icons;

/// The service every item registers with. One per session - ours, since the bar
/// is the only tray host on this desktop.
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

/// The interface an item implements.
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
/// Where an item lives when it registered with its bus name only, which is the
/// usual case and the spec's default.
const ITEM_PATH: &str = "/StatusNotifierItem";

/// The prefix for the host name; the pid makes it unique, and an item that asks
/// "is anybody hosting?" looks for exactly this shape.
const HOST_PREFIX: &str = "org.kde.StatusNotifierHost";

/// The watcher interface, exactly as the StatusNotifierItem specification
/// writes it. `gio` needs the XML to know the method and property signatures -
/// this is what the object is registered from.
const WATCHER_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg name="service" type="s" direction="in"/>
    </method>
    <method name="RegisterStatusNotifierHost">
      <arg name="service" type="s" direction="in"/>
    </method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered">
      <arg name="service" type="s"/>
    </signal>
    <signal name="StatusNotifierItemUnregistered">
      <arg name="service" type="s"/>
    </signal>
    <signal name="StatusNotifierHostRegistered"/>
  </interface>
</node>
"#;

/// How long an item may take to answer a method call. It is a local socket to a
/// process that is running, so a longer wait only means we are talking to
/// something that has stopped listening.
const CALL_TIMEOUT_MS: i32 = 500;

/// The host, once it is up.
///
/// Keep this alive for as long as the bar exists, and call [`Tray::release`] on
/// the way out. `gio::OwnerId` is a plain handle, not a resource that frees
/// itself: dropping it neither gives the name back nor takes the registration
/// (and the closures, and the widgets in them) down with it.
pub struct Tray {
    watcher: RefCell<Option<gio::OwnerId>>,
    host: RefCell<Option<gio::OwnerId>>,
}

struct State {
    /// Set once the session bus answers. The bus is not there when the bar
    /// starts, so nothing here may assume a connection.
    connection: Option<gio::DBusConnection>,
    /// Where the buttons go: a box in the bar's right section.
    container: gtk::Box,
    /// The size an item icon is drawn at, in layout pixels.
    icon_size: i32,
    items: Vec<Item>,
    /// Items whose proxy is still being built (see [`add_item`]). An
    /// application that registers twice in a row would otherwise get two
    /// buttons, because neither registration would see the other's item yet.
    pending: Vec<String>,
}

struct Item {
    /// The bus name and path in the spec's "name/path" spelling: what
    /// `RegisteredStatusNotifierItems` answers with, and the key the item is
    /// looked up by everywhere else.
    registered: String,
    button: gtk::Button,
    proxy: gio::DBusProxy,
}

impl Tray {
    /// Host the tray in `container`.
    ///
    /// Returns immediately: D-Bus is asynchronous, so the icons appear when the
    /// applications answer, which is the only order that cannot block the bar's
    /// start-up.
    pub fn host(container: &gtk::Box, icon_size: i32) -> Tray {
        let state = Rc::new(RefCell::new(State {
            connection: None,
            container: container.clone(),
            icon_size,
            items: Vec::new(),
            pending: Vec::new(),
        }));

        // Owning the watcher name is a request to the bus, not a promise: it can
        // be refused (another tray is running), which is what the last argument
        // reports.
        let watcher = own(
            WATCHER_NAME,
            {
                let state = state.clone();
                move |connection| register_watcher(&state, connection)
            },
            "another tray host already owns it - the tray stays empty",
        );
        // The host name is what items look for to answer "is anybody listening?".
        // Nothing depends on it, so losing it is not worth a word.
        let host = own(&format!("{HOST_PREFIX}-{}", std::process::id()), |_| {}, "");

        Tray {
            watcher: RefCell::new(Some(watcher)),
            host: RefCell::new(Some(host)),
        }
    }

    /// Give both D-Bus names back: the tray is over from here, and another host
    /// may take the session's tray.
    ///
    /// Called on shutdown. The names would also go when the process ends - the
    /// bus drops them - but this is what makes the shutdown path honest, and what
    /// takes the item proxies (and their buttons) down with it.
    pub fn release(&self) {
        if let Some(id) = self.watcher.borrow_mut().take() {
            gio::bus_unown_name(id);
        }
        if let Some(id) = self.host.borrow_mut().take() {
            gio::bus_unown_name(id);
        }
    }
}

/// Ask the session bus for `name` and run `on_acquired` when it is ours.
/// `complaint` is logged if somebody else has it (empty = say nothing).
fn own(
    name: &str,
    on_acquired: impl Fn(&gio::DBusConnection) + 'static,
    complaint: &'static str,
) -> gio::OwnerId {
    gio::bus_own_name(
        gio::BusType::Session,
        name,
        gio::BusNameOwnerFlags::NONE,
        move |connection, _name| on_acquired(&connection),
        // Acquired: `bus_acquired` already ran and there is nothing to add.
        |_, _| {},
        move |_connection: Option<gio::DBusConnection>, name| {
            if !complaint.is_empty() {
                eprintln!("hypr-osd-bar: cannot own {name}: {complaint}");
            }
        },
    )
}

/// Publish the watcher interface on the connection we just got.
fn register_watcher(state: &Rc<RefCell<State>>, connection: &gio::DBusConnection) {
    state.borrow_mut().connection = Some(connection.clone());

    let Ok(node) = gio::DBusNodeInfo::for_xml(WATCHER_XML) else {
        eprintln!("hypr-osd-bar: the tray interface XML is invalid");
        return;
    };
    let Some(interface) = node.interfaces().first() else {
        return;
    };

    let method_state = state.clone();
    let property_state = state.clone();
    let registered = connection
        .register_object(WATCHER_PATH, interface)
        .method_call(
            move |_connection, sender, _path, _interface, method, params, invocation| {
                on_method(&method_state, sender, method, &params, &invocation);
            },
        )
        .property(move |_connection, _sender, _path, _interface, property| {
            on_property(&property_state, property)
        })
        .build();
    if let Err(error) = registered {
        eprintln!("hypr-osd-bar: could not publish {WATCHER_PATH}: {error}");
        return;
    }

    // Tell the bus - and anything watching it - that a host exists now. Items
    // that check before showing themselves look at exactly this.
    let _ = connection.emit_signal(
        None,
        WATCHER_PATH,
        WATCHER_IFACE,
        "StatusNotifierHostRegistered",
        None,
    );
}

/// One method call on the watcher.
fn on_method(
    state: &Rc<RefCell<State>>,
    sender: Option<&str>,
    method: &str,
    params: &glib::Variant,
    invocation: &gio::DBusMethodInvocation,
) {
    match method {
        // "Here I am, draw me." The argument is either a bus name or an object
        // path - the spec allows both, and an application that sends a path means
        // "the bus name I am calling you from".
        "RegisterStatusNotifierItem" => {
            // A method's parameters arrive as the *tuple* the signature
            // describes - `(s)` here, not a bare string - so the argument is
            // taken out of the first slot. (Getting this wrong is invisible until
            // an application registers: it just gets InvalidArgs back.)
            let Some(service) = params
                .child_value(0)
                .get::<String>()
                .filter(|service| !service.is_empty())
            else {
                invocation.clone().return_error(
                    gio::DBusError::InvalidArgs,
                    "RegisterStatusNotifierItem expects a string",
                );
                return;
            };
            let (bus, path) = if service.starts_with('/') {
                (sender.unwrap_or_default().to_string(), service)
            } else {
                (service, ITEM_PATH.to_string())
            };
            // The reply goes out *before* anything else happens with this item.
            // An application may still be sitting inside its own registration
            // call, and it cannot answer a question about itself until it has its
            // answer from us. (`return_value` consumes the invocation, which is
            // how gio models "this call is answered exactly once".)
            invocation.clone().return_value(Some(&().to_variant()));
            add_item(state, &bus, &path);
        }
        // Somebody else wants to be the host. We already are, and the spec has
        // no way to say "no thanks" - so this is acknowledged and ignored.
        "RegisterStatusNotifierHost" => invocation.clone().return_value(Some(&().to_variant())),
        _ => invocation
            .clone()
            .return_error(gio::DBusError::UnknownMethod, "no such method"),
    }
}

/// One property read on the watcher.
///
/// `gio`'s registration takes a closure that always answers with a value, so an
/// unknown property answers with the unit variant instead of an error. Nothing
/// reads them: the three below are the whole interface.
fn on_property(state: &Rc<RefCell<State>>, property: &str) -> glib::Variant {
    let state = state.borrow();
    match property {
        "RegisteredStatusNotifierItems" => {
            let items: Vec<String> = state
                .items
                .iter()
                .map(|item| item.registered.clone())
                .collect();
            items.to_variant()
        }
        "IsStatusNotifierHostRegistered" => true.to_variant(),
        // The protocol version this watcher implements. Nothing reads it in
        // practice, and the reference implementation answers 0.
        "ProtocolVersion" => 0i32.to_variant(),
        _ => ().to_variant(),
    }
}

/// A new item: answer the registration, then build its proxy.
///
/// The proxy is built **asynchronously**, and that is not a nicety. Creating one
/// asks the item for its properties, and an application that registers
/// synchronously is still sitting inside its own `RegisterStatusNotifierItem`
/// call while it does - so a synchronous proxy here would wait for a reply that
/// cannot come until this very method handler returns. That deadlock is what the
/// first version of this module did, and the symptom is an application timing out
/// on registration with no icon to show for it. (`g_dbus_proxy_new` also uses the
/// connection's default 25-second timeout, which would freeze the bar just as
/// thoroughly.)
fn add_item(state: &Rc<RefCell<State>>, bus: &str, path: &str) {
    let Some(connection) = state.borrow().connection.clone() else {
        return;
    };
    let registered = if path == ITEM_PATH {
        bus.to_string()
    } else {
        format!("{bus}{path}")
    };
    {
        let mut state = state.borrow_mut();
        if bus.is_empty()
            || state.items.iter().any(|item| item.registered == registered)
            || state.pending.contains(&registered)
        {
            return;
        }
        state.pending.push(registered.clone());
    }

    let state = state.clone();
    gio::DBusProxy::new(
        &connection,
        gio::DBusProxyFlags::NONE,
        None,
        Some(bus),
        path,
        ITEM_IFACE,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(proxy) => attach_item(&state, &registered, proxy),
            Err(error) => {
                // An item that registered and then vanished (or that refuses to
                // be introspected) gets one line in the log and no button.
                state.borrow_mut().pending.retain(|key| key != &registered);
                eprintln!("hypr-osd-bar: tray item {registered} is not reachable: {error}");
            }
        },
    );
}

/// The item's proxy has arrived: give it a button and start listening to it.
fn attach_item(state: &Rc<RefCell<State>>, registered: &str, proxy: gio::DBusProxy) {
    state.borrow_mut().pending.retain(|key| key != registered);
    let registered = registered.to_string();

    let button = gtk::Button::new();
    button.add_css_class("tray-item");
    button.set_has_frame(false);
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    // Hidden until an icon has been resolved, so the bar never grows a gap for
    // an item that turns out to have nothing to draw.
    button.set_visible(false);
    let image = gtk::Image::new();
    button.set_child(Some(&image));
    state.borrow().container.append(&button);

    // Clicks are *passed on* rather than acted on: the item decides what its own
    // icon means. Buttons 1 and 2 ask for the primary and secondary action,
    // button 3 asks the item to show its menu wherever it likes.
    {
        let proxy = proxy.clone();
        let registered = registered.clone();
        button.connect_clicked(move |_| call(&proxy, &registered, "Activate"));
    }
    for (button_number, method) in [
        (gdk::BUTTON_MIDDLE, "SecondaryActivate"),
        (gdk::BUTTON_SECONDARY, "ContextMenu"),
    ] {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(button_number);
        let proxy = proxy.clone();
        let registered = registered.clone();
        gesture.connect_pressed(move |_, _, _, _| call(&proxy, &registered, method));
        button.add_controller(gesture);
    }

    // An item redraws itself by changing its properties or by announcing one of
    // the `New*` signals; both paths exist in the wild, so both are listened to.
    // `connect_local` is what makes that possible from a closure that holds GTK
    // widgets: these two signals carry no generated binding, and the generated
    // property-notify helpers want a `Send + Sync` closure, which no closure
    // touching widgets can be.
    {
        let state = state.clone();
        let registered = registered.clone();
        proxy.connect_local("g-properties-changed", false, move |_| {
            refresh_item(&state, &registered);
            None
        });
    }
    {
        let state = state.clone();
        let registered = registered.clone();
        proxy.connect_local("g-signal", false, move |values| {
            // The payload is (sender, signal name, parameters).
            let signal = values.get(1).and_then(|value| value.get::<String>().ok());
            if matches!(
                signal.as_deref(),
                Some("NewIcon" | "NewStatus" | "NewTitle" | "NewToolTip" | "NewAttentionIcon")
            ) {
                refresh_item(&state, &registered);
            }
            None
        });
    }
    // An item that exits (or crashes) leaves only its proxy behind, and the proxy
    // is who notices: no name owner left means there is nothing to draw, so the
    // button goes with it.
    {
        let state = state.clone();
        let registered = registered.clone();
        proxy.connect_local("notify::g-name-owner", false, move |values| {
            let gone = values
                .first()
                .and_then(|value| value.get::<gio::DBusProxy>().ok())
                .is_none_or(|proxy| proxy.name_owner().is_none());
            if gone {
                remove_item(&state, &registered);
            }
            None
        });
    }

    state.borrow_mut().items.push(Item {
        registered: registered.clone(),
        button,
        proxy,
    });
    refresh_item(state, &registered);

    // Tell the bus, in case somebody else is interested in what is on the tray.
    if let Some(connection) = state.borrow().connection.clone() {
        let _ = connection.emit_signal(
            None,
            WATCHER_PATH,
            WATCHER_IFACE,
            "StatusNotifierItemRegistered",
            Some(&(registered,).to_variant()),
        );
    }
}

/// Drop an item whose application has gone.
fn remove_item(state: &Rc<RefCell<State>>, registered: &str) {
    let mut state = state.borrow_mut();
    let Some(index) = state
        .items
        .iter()
        .position(|item| item.registered == registered)
    else {
        return;
    };
    let item = state.items.remove(index);
    state.container.remove(&item.button);
}

/// Redraw one item from what its proxy currently caches.
fn refresh_item(state: &Rc<RefCell<State>>, registered: &str) {
    let state = state.borrow();
    let Some(item) = state
        .items
        .iter()
        .find(|item| item.registered == registered)
    else {
        return;
    };
    item.button
        .set_tooltip_text(tooltip(&item.proxy).as_deref());
    // A tinted background for the one state the spec asks a host to show: the
    // item has something to say (a new message, an unread count).
    if matches!(status(&item.proxy).as_deref(), Some("NeedsAttention")) {
        item.button.add_css_class("needs-attention");
    } else {
        item.button.remove_css_class("needs-attention");
    }
    if let Some(image) = item.button.child().and_downcast::<gtk::Image>() {
        draw_icon(&image, &item.proxy, state.icon_size);
    }
    // Only now, with an icon resolved: see `add_item`.
    item.button.set_visible(true);
}

/// Put an item's icon on the image, pixmap first.
///
/// A pixmap wins over a name on purpose: it is what the application drew, at the
/// size it chose, whereas a name is resolved against *this* theme and can come
/// out as something the application never intended (or as nothing at all).
fn draw_icon(image: &gtk::Image, proxy: &gio::DBusProxy, size: i32) {
    let name = if matches!(status(proxy).as_deref(), Some("NeedsAttention")) {
        property(proxy, "AttentionIconName").or_else(|| property(proxy, "IconName"))
    } else {
        property(proxy, "IconName")
    };

    if let Some(texture) =
        largest_pixmap(proxy).and_then(|(width, height, bytes)| texture(width, height, &bytes))
    {
        image.set_paintable(Some(&texture));
        image.set_pixel_size(size);
        return;
    }
    if let Some(paintable) = name.and_then(|name| icons::by_name(&name, size)) {
        image.set_paintable(Some(&paintable));
        image.set_pixel_size(size);
        return;
    }
    // Nothing to draw: the generic missing-image icon at least says "there is an
    // application here and it has no icon", which an empty box does not.
    image.set_icon_name(Some("image-missing"));
    image.set_pixel_size(size);
}

/// The tooltip text: the item's title, or its id when it has no title.
fn tooltip(proxy: &gio::DBusProxy) -> Option<String> {
    property(proxy, "Title")
        .filter(|title| !title.trim().is_empty())
        .or_else(|| property(proxy, "Id"))
}

fn status(proxy: &gio::DBusProxy) -> Option<String> {
    property(proxy, "Status")
}

/// One string property, straight out of what the proxy already has.
fn property(proxy: &gio::DBusProxy, name: &str) -> Option<String> {
    let text = proxy.cached_property(name)?.get::<String>()?;
    (!text.trim().is_empty()).then_some(text)
}

/// The biggest pixmap an item offered, with its pixels.
///
/// Items send the same icon at several sizes so a host can pick the one that
/// matches its bar; taking the largest and scaling down gives the sharpest result
/// on a HiDPI screen, which is what this desktop has.
fn largest_pixmap(proxy: &gio::DBusProxy) -> Option<(i32, i32, Vec<u8>)> {
    let pixmaps = proxy
        .cached_property("IconPixmap")?
        .get::<Vec<(i32, i32, Vec<u8>)>>()?;
    pixmaps
        .into_iter()
        .filter(|(width, height, bytes)| {
            *width > 0 && *height > 0 && bytes.len() >= (*width as usize * *height as usize * 4)
        })
        .max_by_key(|(width, height, _)| *width * *height)
}

/// A StatusNotifierItem pixmap is ARGB32 in *network* byte order - the bytes on
/// the wire are A, R, G, B - while a GDK memory texture wants RGBA in memory.
/// This is the shuffle between the two.
fn texture(width: i32, height: i32, bytes: &[u8]) -> Option<gdk::Texture> {
    let expected = width as usize * height as usize * 4;
    if expected == 0 || bytes.len() < expected {
        return None;
    }
    let rgba: Vec<u8> = bytes[..expected]
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[1], pixel[2], pixel[3], pixel[0]])
        .collect();
    let texture = gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(rgba),
        (width * 4) as usize,
    );
    Some(texture.upcast())
}

/// Pass a click on to the item.
///
/// Asynchronous, like the rest of this module: a click must never wait for an
/// application to answer, and a wedged item must not be able to freeze the bar. A
/// failure is reported with a name, because "the tray icon did nothing" is
/// otherwise impossible to attribute.
///
/// The coordinates say where in the icon the click landed; every host in practice
/// passes `0, 0` - the item knows where its own icon is - and doing the same keeps
/// us out of trouble with the items that do look at them.
fn call(proxy: &gio::DBusProxy, registered: &str, method: &str) {
    let registered = registered.to_string();
    // A second copy for the callback: the call itself needs the name only for
    // the duration of the call, and the complaint below outlives it.
    let complained = method.to_string();
    proxy.call(
        method,
        Some(&(0i32, 0i32).to_variant()),
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
        gio::Cancellable::NONE,
        move |result| {
            if let Err(error) = result {
                // An item that does not implement one of the three methods is
                // allowed to say so (KDE's own examples do); that is not worth a
                // fuss, but a wedged item is - and this is the only place that
                // can say which it was.
                eprintln!(
                    "hypr-osd-bar: tray item {registered} did not answer {complained}: {error}"
                );
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_pixels_are_shuffled_into_rgba() {
        // A single opaque red pixel: A=ff, R=ff, G=00, B=00 -> R,G,B,A.
        let argb = [0xffu8, 0xff, 0x00, 0x00];
        let rgba: Vec<u8> = argb
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[1], pixel[2], pixel[3], pixel[0]])
            .collect();
        assert_eq!(rgba, vec![0xff, 0x00, 0x00, 0xff]);
    }

    #[test]
    fn a_pixmap_that_is_too_short_for_its_size_is_refused() {
        // 2x2 needs 16 bytes; 8 is not a picture, it is a truncated one.
        assert!(texture(2, 2, &[0u8; 8]).is_none());
        assert!(texture(0, 2, &[]).is_none());
        // Trailing bytes beyond the image are ignored rather than fatal.
        assert!(texture(1, 1, &[0xff, 0x00, 0x00, 0xff, 0x11]).is_some());
    }
}
