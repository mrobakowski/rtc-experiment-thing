#[macro_use]
extern crate stdweb_derive;
#[macro_use]
extern crate stdweb;

use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
};

use rand::{
    distributions::{Alphanumeric, Distribution},
    thread_rng,
};

use stdweb::traits::*;
use stdweb::unstable::TryInto;

use stdweb::web::{
    document,
    window,
    WebSocket,
    event::{SocketMessageEvent, SocketOpenEvent, ConcreteEvent},
    EventListenerHandle,
    EventTarget
};

use serde_json::Value;
use stdweb::Reference;
use stdweb::private::ConversionError;
use stdweb::serde::Serde;

type Error = Box<dyn std::error::Error>;
type Result<T, E = Error> = std::result::Result<T, E>;

pub fn main() -> Result<()> {
    stdweb::initialize();

    let document = document();
    let body = document.body().ok_or("no body, help")?;

    let val = document.create_element("p")?;
    val.set_text_content("Hello from Rust, WebAssembly, and stdweb!");
    body.append_child(&val);

    let room_name = {
        let mut u: String = window().location().ok_or("no location, rip")?.hash()?;

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

    let _room = Room::join(room_name, MyGame)?;

    stdweb::event_loop();

    Ok(())
}

type WebSocketAndListeners = EventTargetWrapper<WebSocket>;

impl WebSocketAndListeners {
    fn close_and_cleanup(mut self) {
        self.inner.close();
        for listener in self.listeners.drain(..) {
            listener.remove()
        }
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

    fn init(&mut self, _room: &mut Room<Self>) where Self: Sized {}
    fn on_message(&mut self, _m: Self::Message) {}
    fn on_connected(&mut self) {}
    fn on_disconnected(&mut self) {}
}

impl<G: Game + 'static> Room<G> {
    pub fn join(name: impl Into<String>, game: G) -> Result<&'static RefCell<Self>> {
        let this = Room {
            name: name.into(),
            game,
            web_socket: None,
            is_host: false,
        };

        // if you love something, set it free
        let this: &'static RefCell<Room<G>> = Box::leak(Box::new(RefCell::new(this)));
        let mut host_ws = WebSocketAndListeners::new(WebSocket::new(
            &format!("ws://localhost:8001?user={}-host", this.borrow().name))?);
        let mut first_message = true;

        host_ws.on(move |e: SocketMessageEvent| {
            if first_message {
                console!(log, "host_on_message 1st message");

                let mut this = this.borrow_mut();
                if e.data().into_text().filter(|s| *s == "The username is taken").is_some() {
                    console!(log, "The username was taken! We're not the host!");
                    this.web_socket.take().unwrap().close_and_cleanup();
                    console!(log, "after websocket closed");
                    this.join_client().unwrap();
                    console!(log, "after client joined");
                } else {
                    console!(log, "we're probably the host");
                    this.init_host();
                    this.host_on_message(e).unwrap();
                }

                first_message = false;
            } else {
                this.borrow_mut().host_on_message(e).unwrap();
            }
        });

        host_ws.on(move |_: SocketOpenEvent| {
            this.borrow().web_socket.as_ref().unwrap()
                .send_text(r#"{"protocol": "one-to-self", "type": "host-confirmation-message"}"#)
                .expect("Couldn't send self-message");
        });

        this.borrow_mut().web_socket = Some(host_ws);

        Ok(this)
    }

    fn join_client(&mut self) -> Result<()> {
        self.is_host = false;

        document().set_title("Client: RTC Experiment");
        let client_username: String = Alphanumeric.sample_iter(&mut thread_rng()).take(5).collect();

        let host_name = format!("{}-{}", self.name, "host");
        let self_name = format!("{}-{}", self.name, client_username);

        let ws = WebSocketAndListeners::new(WebSocket::new(&format!("ws://localhost:8001?user={}", self_name))?);
        let rtc = RtcPeerConnection::new()?;
        let host_connection = rtc.create_data_channel("hostConnection");

        init_rtc_client(&rtc, &ws, &host_name, &self_name);

        self.web_socket = Some(ws);

        Ok(())
    }

    fn init_host(&mut self) {
        self.is_host = true;

        document().set_title("Host: RTC Experiment");
    }

    fn host_on_message(&mut self, m: SocketMessageEvent) -> Result<()> {
        console!(log, "host: received message");
        if let Some(parsed_message) = maybe_rtc_offer(&m) {
            let rtc = RtcPeerConnection::new()?;
            let host_name = format!("{}-{}", self.name, "host");
            let client_name = &parsed_message["from"].as_str().ok_or("no from field")?;
            let ws = &*self.web_socket.as_ref().unwrap();
            init_rtc_host(&rtc, ws, &host_name, &client_name, &parsed_message["payload"]);
        }

        Ok(())
    }
}

fn maybe_rtc_offer(m: &SocketMessageEvent) -> Option<Value> {
    let parsed_message: Value = serde_json::from_str(&m.data().into_text()?).ok()?;

    if parsed_message["type"].as_str()? == "rtc-offer" {
        Some(parsed_message)
    } else {
        None
    }
}


struct EventTargetWrapper<T> {
    inner: T,
    listeners: Vec<EventListenerHandle>,
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

impl<T> EventTargetWrapper<T> where T: IEventTarget {
    fn new(inner: T) -> Self { EventTargetWrapper { inner, listeners: vec![] } }

    fn on<F, A>(&mut self, f: F) where A: ConcreteEvent, F: FnMut(A) + 'static {
        let handle = self.add_event_listener(f);
        self.listeners.push(handle);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, ReferenceType)]
#[reference(instance_of = "RTCPeerConnection")]
#[reference(subclass_of(EventTarget))]
pub struct RtcPeerConnection(Reference);

impl RtcPeerConnection {
    fn new() -> Result<RtcPeerConnection, ConversionError> {
        js!(return new RTCPeerConnection();).try_into()
    }

    fn create_data_channel(&self, label: &str) -> stdweb::Value {
        let res = js! { return @{self}.createDataChannel(@{label}); };
        res
    }
}

fn init_rtc_client(rtc: &RtcPeerConnection, ws: &WebSocket, host_name: &str, self_name: &str) {
    js! { @(no_return)
        window.initRtc.client(@{rtc}, @{ws}, @{host_name}, @{self_name})
    }
}

fn init_rtc_host(rtc: &RtcPeerConnection, ws: &WebSocket, host_name: &str, client_name: &str, offer: &Value) {
    js! { @(no_return)
        window.initRtc.host(@{rtc}, @{ws}, @{host_name}, @{client_name}, @{Serde(offer)})
    }
}
