use cfg_if::cfg_if;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

cfg_if! {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function to get better error messages if we ever panic.
    if #[cfg(feature = "console_error_panic_hook")] {
        use console_error_panic_hook::set_once as set_panic_hook;
    } else {
        #[inline]
        fn set_panic_hook() {}
    }
}

cfg_if! {
    // When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
    // allocator.
    if #[cfg(feature = "wee_alloc")] {
        #[global_allocator]
        static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
    }
}

macro_rules! el {
    ($storage:ident; $e: expr) => {{
        let c = Closure::<Fn(JsValue) -> ()>::new($e);
        $storage.push(c);
        $storage.last().unwrap().as_ref().unchecked_ref()
    }};
}

// Called by our JS entry point to run the example
#[wasm_bindgen]
pub fn run() -> Result<(), JsValue> {
    // If the `console_error_panic_hook` feature is enabled this will set a panic hook, otherwise
    // it will do nothing.
    set_panic_hook();
    console_log::init_with_level(log::Level::Debug).expect("could not initialize logging");

    // Use `web_sys`'s global `window` function to get a handle on the global
    // window object.
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    let body = document.body().expect("document should have a body");

    // Manufacture the element we're gonna append
    let val = document.create_element("p")?;
    val.set_inner_html("Hello from Rust, WebAssembly, and Parcel!");

    body.append_child(&val)?;

    use web_sys::console;

    let mut to_forget = Vec::new();

    let username = {
        let mut u: String = window.location().hash()?;

        if u.is_empty() {
            "Player1".into()
        } else {
            u.remove(0);
            u
        }
    };

    log::info!("username is {}", username);

    let ws = web_sys::WebSocket::new(&format!("ws://localhost:8000?user={}", username))?;
    log::info!("hello!");
    ws.add_event_listener_with_callback("message", el!(to_forget; |e: JsValue| {
        log::debug!("on_message");
        console::log_1(&e);
    }))?;

    let ws_clone = ws.clone();

    ws.add_event_listener_with_callback("open", el!(to_forget; move |e: JsValue| {
        log::debug!("on_open");
        console::log_1(&e);

        ws_clone.send_with_str(&format!(r##"{{
            "username": "{}",
            "protocol": "one-to-all",
            "dupa": "xDDDD"
        }}"##, username)).unwrap();
    }))?;

    for closure in to_forget {
        closure.forget();
    }

    Ok(())
}
