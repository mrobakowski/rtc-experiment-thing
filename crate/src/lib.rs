use cfg_if::cfg_if;
use wasm_bindgen::{
    prelude::*,
    JsCast,
    convert::{FromWasmAbi, ReturnWasmAbi},
};
use web_sys::{
    console,
    WebSocket,
    MessageEvent,
    Event,
    RtcPeerConnection,
    EventTarget,
};
use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
};
use js_sys::{
    Reflect,
    JSON,
    JsString,
};
use rand::{
    distributions::{Alphanumeric, Distribution},
    thread_rng,
};

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

    let room = Room::join(room_name, MyGame)?;

    Ok(())
}

type WebSocketAndListeners = EventTargetWrapper<WebSocket>;


impl WebSocketAndListeners {
    fn close_and_cleanup(self) {
        self.inner.close().expect("Couldn't close the websocket");
        // the listeners get dropped and cleaned up in their drop impl
    }
}

struct Room<G: 'static> {
    game: G,
    name: String,
    web_socket: Option<WebSocketAndListeners>,
    is_host: bool,
}

trait Game {
    type Message;

    fn init(&mut self, room: &mut Room<Self>) where Self: Sized {}
    fn on_message(&mut self, m: Self::Message) {}
    fn on_connected(&mut self) {}
    fn on_disconnected(&mut self) {}
}


impl<G: Game + 'static> Room<G> {
    pub fn join(name: impl Into<String>, game: G) -> Result<&'static RefCell<Self>, JsValue> {
        let this = Room {
            name: name.into(),
            game,
            web_socket: None,
            is_host: false,
        };

        // if you love something, set it free
        let this: &'static RefCell<Room<G>> = Box::leak(Box::new(RefCell::new(this)));

        let mut host_ws = EventTargetWrapper::new(WebSocket::new(&format!("ws://localhost:8000?user={}-host", this.borrow().name))?);

        let mut first_message = true;

        host_ws.on("message", move |e: MessageEvent| {
            if first_message {
                log::debug!("host_on_message 1st message");
                console::log_1(&e);

                let mut this = this.borrow_mut();
                if JsString::try_from(&e.data()).filter(|s| *s == "The username is taken").is_some() {
                    log::debug!("The username was taken! We're not the host!");
                    this.web_socket.take().unwrap().close_and_cleanup();
                    this.join_client().unwrap();
                } else {
                    log::debug!("we're probably the host");
                    this.init_host();
                    this.host_on_message(e);
                }

                first_message = false;
            } else {
                this.borrow_mut().host_on_message(e);
            }
        })?;

        host_ws.on("open", move |e: Event| {
            this.borrow().web_socket.as_ref().unwrap()
                .send_with_str(r#"{"protocol": "one-to-self", "type": "host-confirmation-message"}"#)
                .expect("Couldn't send self-message");
        })?;

        this.borrow_mut().web_socket = Some(host_ws);

        Ok(this)
    }

    fn join_client(&mut self) -> Result<(), JsValue> {
        self.is_host = false;

        web_sys::window().expect("no global `window` exists").document().expect("document doesn't exist").set_title("Client: RTC Experiment");
        let client_username: String = Alphanumeric.sample_iter(&mut thread_rng()).take(5).collect();

        let host_name = format!("{}-{}", self.name, "host");
        let self_name = format!("{}-{}", self.name, client_username);

        let ws = WebSocketAndListeners::new(WebSocket::new(&format!("ws://localhost:8000?user={}", self_name))?);
        let rtc = EventTargetWrapper::new(RtcPeerConnection::new()?);
        let host_connection = rtc.create_data_channel("hostConnection");

        init_rtc_client(&rtc, &ws, &host_name, &self_name)?;

        self.web_socket = Some(ws);

        Ok(())
    }

    fn init_host(&mut self) {
        self.is_host = true;

        web_sys::window().expect("no global `window` exists").document().expect("document doesn't exist").set_title("Host: RTC Experiment");
    }

    fn host_on_message(&mut self, m: MessageEvent) {
        log::debug!("host: received message");
        console::log_1(&m);
        if let Some(parsed_message) = maybe_etc_offer(&m) {
            let rtc = RtcPeerConnection::new().unwrap();
            let host_name = format!("{}-{}", self.name, "host");
            let client_name = Reflect::get(&parsed_message, &JsValue::from_str("from")).unwrap().as_string().unwrap();
            let ws = &*self.web_socket.as_ref().unwrap();
            init_rtc_host(&rtc, ws, &host_name, &client_name, &Reflect::get(&parsed_message, &JsValue::from_str("payload")).unwrap()).unwrap();
        }
    }
}

fn maybe_etc_offer(m: &MessageEvent) -> Option<JsValue> {
    let parsed_message = JSON::parse(&m.data().as_string()?).ok()?;
    let typ = &JsValue::from_str("type");
    if Reflect::has(&parsed_message, typ).ok()? && Reflect::get(&parsed_message, typ).ok()?.as_string()? == "rtc-offer" {
        Some(parsed_message)
    } else {
        None
    }
}


struct EventTargetWrapper<T> {
    inner: T,
    listeners: Vec<Box<dyn Drop>>,
}

impl<T> Deref for EventTargetWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for EventTargetWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> EventTargetWrapper<T> where T: Deref<Target=EventTarget> {
    fn new(inner: T) -> Self { EventTargetWrapper { inner, listeners: vec![] } }

    fn on<F, A, R>(&mut self, event: &str, f: F) -> Result<(), JsValue>
        where F: (FnMut(A) -> R) + 'static,
              A: FromWasmAbi + 'static,
              R: ReturnWasmAbi + 'static {
        let closure = Closure::<FnMut(A) -> R>::new(f);
        self.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
        self.listeners.push(Box::new(closure));
        Ok(())
    }
}

#[wasm_bindgen(module = "./../../js/rtc")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn init_rtc_client(rtc: &RtcPeerConnection, ws: &WebSocket, host_name: &str, self_name: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(catch)]
    fn init_rtc_host(rtc: &RtcPeerConnection, ws: &WebSocket, host_name: &str, client_name: &str, offer: &JsValue) -> Result<(), JsValue>;
}
