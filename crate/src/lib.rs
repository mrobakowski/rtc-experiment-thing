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

    let room = Room::join(room_name, MyGame)?;

    Ok(())
}

type WebSocketAndListeners = EventTargetWrapper<WebSocket>;

use wasm_bindgen::convert::{FromWasmAbi, ReturnWasmAbi};

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
    this: &'static RefCell<Self>,
}

trait Game {
    type Message;

    fn init(&mut self, room: &mut Room<Self>) where Self: Sized {}
    fn on_message(&mut self, m: Self::Message) {}
    fn on_connected(&mut self) {}
    fn on_disconnected(&mut self) {}
}

use web_sys::{MessageEvent, Event, RtcPeerConnection, RtcPeerConnectionIceEvent};
use rand::distributions::Alphanumeric;
use rand::thread_rng;
use rand::distributions::Distribution;

impl<G: Game + 'static> Room<G> {
    pub fn join(name: impl Into<String>, game: G) -> Result<&'static RefCell<Self>, JsValue> {
        use js_sys::JsString;

        let this = Room {
            name: name.into(),
            game,
            web_socket: None,
            is_host: false,
            this: unsafe { std::mem::uninitialized() },
        };

        // if you love something, set it free
        let this: &'static RefCell<Room<G>> = Box::leak(Box::new(RefCell::new(this)));

        // forget the uninitialized value to prevent Ancient Ones taking over this code
        std::mem::forget(std::mem::replace(&mut this.borrow_mut().this, this));


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
                    this.join_client();
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
        let mut ws = WebSocketAndListeners::new(WebSocket::new(&format!("ws://localhost:8000?user={}-{}", self.name, client_username))?);
        let mut rtc = EventTargetWrapper::new(RtcPeerConnection::new()?);
        let host_connection = rtc.create_data_channel("hostConnection");

        rtc.on("icecandidate", |e: RtcPeerConnectionIceEvent| {
            // send ice candidate to host via websocket
        })?;

        let mut closures = vec![];

//        localConnection.createOffer()
//            .then(offer => localConnection.setLocalDescription(offer))
//            .then(() => remoteConnection.setRemoteDescription(localConnection.localDescription))
//            .then(() => remoteConnection.createAnswer())
//            .then(answer => remoteConnection.setLocalDescription(answer))
//            .then(() => localConnection.setRemoteDescription(remoteConnection.localDescription))
//            .catch(handleCreateDescriptionError);

        macro_rules! c {
            ($x: expr) => {{
                let closure = Closure::<dyn FnMut(JsValue) -> _>::new($x);
                closures.push(closure);
                unsafe { std::mem::transmute(closures.last().unwrap()) } // TODO: ???
            }};
        }

        rtc.create_offer()
            .then(c!(|offer: JsValue| rtc.inner.set_local_description(offer.unchecked_ref())))
            .then(unimplemented!());


        let connect_payload = "";

        let this: &'static RefCell<Room<G>> = self.this;
        ws.on("open", move |e: Event| {
            this.borrow().web_socket.as_ref().unwrap()
                .send_with_str(&format!(r#"{{"protocol": "one-to-one", "to": "{}-host", "type": "rtc-request", "payload": "{}"}}"#, this.borrow().name, connect_payload))
        })?;

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
    }
}

use web_sys::EventTarget;
use std::ops::Deref;
use std::ops::DerefMut;

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
