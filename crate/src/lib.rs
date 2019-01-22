use cfg_if::cfg_if;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::console;
use web_sys::WebSocket;
use std::cell::RefCell;

cfg_if! {
    if #[cfg(feature = "console_error_panic_hook")] {
        use console_error_panic_hook::set_once as set_panic_hook;
    } else {
        #[inline]
        fn set_panic_hook() {}
    }
}

cfg_if! {
    if #[cfg(feature = "wee_alloc")] {
        #[global_allocator]
        static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
    }
}

#[wasm_bindgen]
pub fn run() -> Result<(), JsValue> {
    set_panic_hook();

    console_log::init_with_level(log::Level::Debug)
        .expect("could not initialize logging");

    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    let body = document.body().expect("document should have a body");

    // Manufacture the element we're gonna append
    let val = document.create_element("p")?;
    val.set_inner_html("Hello from Rust, WebAssembly, and Webpack!");

    body.append_child(&val)?;

    let room_name = {
        let mut u: String = window.location().hash()?;

        if u.is_empty() {
            "aRoom".into()
        } else {
            u.remove(0);
            u
        }
    };

    struct MyGame;
    impl Game for MyGame {
        type Message = ();
    }

    let room = Room::new(room_name, MyGame);

    room.join()?;

    Ok(())
}

struct WebSocketAndListeners {
    socket: WebSocket,
    listeners: Vec<Box<dyn Drop>>
}

use wasm_bindgen::convert::{FromWasmAbi, ReturnWasmAbi};

impl WebSocketAndListeners {
    fn close_and_cleanup(self) {
        self.socket.close().expect("Couldn't close the websocket");
        // the listeners get dropped and cleaned up in their drop impl
    }

    fn on<F, A, R>(&mut self, event: &str, f: F) -> Result<(), JsValue>
        where F: (FnMut(A) -> R) + 'static,
              A: FromWasmAbi + 'static,
              R: ReturnWasmAbi + 'static {
        let closure = Closure::<FnMut(A) -> R>::new(f);
        self.socket.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
        self.listeners.push(Box::new(closure));
        Ok(())
    }
}

struct Room<G> {
    game: G,
    name: String,
    web_socket: Option<WebSocketAndListeners>,
    is_host: bool
}

trait Game {
    type Message;

    fn init(&mut self, room: &mut Room<Self>) where Self: Sized {}
    fn on_message(&mut self, m: Self::Message) {}
    fn on_connected(&mut self) {}
    fn on_disconnected(&mut self) {}
}

use web_sys::{MessageEvent, Event};

impl<G: Game + 'static> Room<G> {
    pub fn new(name: impl Into<String>, game: G) -> Room<G> {
        Room {
            name: name.into(),
            game,
            web_socket: None,
            is_host: false
        }
    }

    pub fn join(self) -> Result<(), JsValue> {
        use js_sys::JsString;

        // if you love something, set it free
        let this: &'static RefCell<Room<G>> =
            Box::leak(Box::new(RefCell::new(self)));

        let host_ws = WebSocket::new(&format!("ws://localhost:8000?user={}-host", this.borrow().name))
            .expect("cannot open websocket");

        let mut first_message = true;

        let mut host_ws = WebSocketAndListeners {
            socket: host_ws,
            listeners: vec![]
        };

        host_ws.on("message", move |e: MessageEvent| {
            if first_message {
                log::debug!("host_on_message 1st message");
                console::log_1(&e);

                let mut this = this.borrow_mut();
                if JsString::try_from(&e.data()).filter(|s| *s == "The username is taken").is_some() {
                    log::debug!("The username was taken! We're not the host!");
                    this.web_socket.take().unwrap().close_and_cleanup();
                    this.join_client();
                } else {
                    log::debug!("we're proooobably the host");
                    this.host_on_message(e);
                }

                first_message = false;
            } else {
                log::debug!("host_on_message not first message");
                console::log_1(&e);

                log::debug!("I sure hope we're the host");
                this.borrow_mut().host_on_message(e);
            }
        })?;

        host_ws.on("open", move |e: Event| {
            this.borrow().web_socket.as_ref().unwrap().socket.send_with_str(r#"{
                "protocol": "one-to-self",
                "type": "host-confirmation-message"
            }"#).expect("Couldn't send self-message");
        })?;

        this.borrow_mut().web_socket = Some(host_ws);

        Ok(())
    }

    fn join_client(&mut self) {

    }

    fn host_on_message(&mut self, m: MessageEvent) {}
}
