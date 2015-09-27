
extern crate mio;
extern crate http_muncher;

#[macro_use]
extern crate log;
extern crate syslog;


use http_muncher::{Parser, ParserHandler};

use std::net::SocketAddr;
use mio::tcp::*;
use mio::*;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::fmt;

use log::{LogRecord, LogLevel, LogMetadata};

struct LoggerFacade
{
    log : Box<syslog::Logger>
}
impl LoggerFacade {
    pub fn init() -> Result<(), log::SetLoggerError> {
        let syslog = match syslog::unix(syslog::Facility::LOG_USER) {
            Ok(log) => log,
            Err(e) => panic!("Can't attach to SYSLOG {}", e)
        };
        log::set_logger(|max_log_level| { 
            max_log_level.set(log::LogLevelFilter::Info);
            Box::new(LoggerFacade{log : syslog}) 
        })
    }
}
impl log::Log for LoggerFacade {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        metadata.level() <= LogLevel::Info
    }

    fn log(&self, record: &LogRecord) {
        if self.enabled(record.metadata()) {
            self.log.send_3164(syslog::Severity::LOG_INFO, format!("{} - {}", record.level(), record.args())).unwrap();
        }
    }
}

extern crate sha1;
extern crate rustc_serialize;

use rustc_serialize::base64::{ToBase64, STANDARD};

fn gen_key(key: &String) -> String {
    let mut m = sha1::Sha1::new();
    let mut buf = [0u8; 20];

    m.update(key.as_bytes());
    m.update("258EAFA5-E914-47DA-95CA-C5AB0DC85B11".as_bytes());

    m.output(&mut buf);

    return buf.to_base64(STANDARD);
}

struct HttpParser {
    current_key: Option<String>,
    headers: Rc<RefCell<HashMap<String, String>>>
}

impl ParserHandler for HttpParser {
    fn on_header_field(&mut self, s: &[u8]) -> bool {
        self.current_key = Some(std::str::from_utf8(s).unwrap().to_string());
        true
    }

    fn on_header_value(&mut self, s: &[u8]) -> bool {
        self.headers.borrow_mut().insert(self.current_key.clone().unwrap(), std::str::from_utf8(s).unwrap().to_string());
        true
    }

    fn on_headers_complete(&mut self) -> bool {
        false
    }
}

const SERVER_TOKEN: Token = Token(1);

#[derive(PartialEq)]
enum ClientState {
    AwaitingHandshake,
    HandshakeResponse,
    Connected
}

struct WebSocketClient{
    socket: TcpStream,
    http_parser: Parser<HttpParser>,
    headers: Rc<RefCell<HashMap<String, String>>>,
    interest: EventSet,
    state: ClientState,
}

impl WebSocketClient{
    fn new(socket: TcpStream) -> WebSocketClient {
        let headers = Rc::new(RefCell::new(HashMap::new()));
        WebSocketClient {
            socket: socket,
            http_parser: Parser::request(HttpParser { current_key : None, headers : headers.clone() }),
            headers: headers.clone(),
            interest: EventSet::readable(),
            state: ClientState::AwaitingHandshake,
        }
    }
    fn read(&mut self) {
        loop {
            let mut buf = [0; 2048];
            match self.socket.try_read(&mut buf) {
                Err(e) => {
                    error!("Error while reading socket: {:?}", e);
                    return
                },
                Ok(None) =>
                {
                    info!("Client socket has no data");
                    // Socket buffer has got no more bytes.
                    break
                }
                Ok(Some(len)) => {
                    info!("Has readed {} bytes", &len);
                    self.http_parser.parse(&buf[0..len]);
                    if self.http_parser.is_upgrade() {
                        // Change the current state
                        self.state = ClientState::HandshakeResponse;
                        // Change current interest to `Writable`
                        self.interest.remove(EventSet::readable());
                        self.interest.insert(EventSet::writable());
                        break;
                    }
                }
            }
        }
    }

    fn write(&mut self) {
        // Get the headers HashMap from the Rc<RefCell<...>> wrapper:
        let headers = self.headers.borrow();

        // Find the header that interests us, and generate the key from its value:
        let response_key = gen_key(&headers.get("Sec-WebSocket-Key").unwrap());

        // We're using special function to format the string.
        // You can find analogies in many other languages, but
        // in Rust it's performed
        // at the compile time with the power of macros.
        // We'll discuss it in the next part sometime.
        let response = fmt::format(format_args!("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nSec-WebSocket-Accept:{}\r\nUpgrade:websocket\r\n\r\n",response_key));
        self.socket.try_write(response.as_bytes()).unwrap();
        self.state = ClientState::Connected;
        self.interest.remove(EventSet::writable());
        self.interest.insert(EventSet::readable());
    }
}

struct WebSocketServer {
    socket: TcpListener,
    clients: HashMap<Token, WebSocketClient>,
    counter: usize,
}
impl WebSocketServer{
    pub fn new() -> Self {
        let server = TcpSocket::v4().unwrap();
        let address = "0.0.0.0:10000".parse::<SocketAddr>().unwrap();
        server.bind(&address).unwrap();
        let server = server.listen(128).unwrap();
        WebSocketServer {
            socket : server,
            clients : HashMap::new(),
            counter : 1,
        }
    }
}
impl Handler for WebSocketServer{
    type Timeout = usize;
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<WebSocketServer>, token: Token, events: EventSet)  {
        info!("WebSocketServer.ready(...): Invoked.");
        if events.is_readable() {
            match token {
                SERVER_TOKEN => {
                    let socket = match self.socket.accept() {
                        Err(e) => {
                            error!("Accept error: {}", e);
                            return;
                        },
                        Ok(None) => {
                            error!("Ok(None) is not acceptable here.");
                            unreachable!("Accept has returned 'None'");
                        },
                        Ok(Some(sock)) => sock
                    };

                    self.counter += 1;
                    let token = Token(self.counter);
                    self.clients.insert(token, WebSocketClient::new(socket));
                    event_loop.register_opt(&self.clients[&token].socket, token, EventSet::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
                },

                token => {
                    let mut client = self.clients.get_mut(&token).unwrap();
                    client.read();
                    event_loop.reregister(&client.socket, token, client.interest, PollOpt::edge() | PollOpt::oneshot()).unwrap();
                },          
            }
        }
        if events.is_writable() {
            let mut client = self.clients.get_mut(&token).unwrap();
            client.write();
            event_loop.reregister(&client.socket, token, client.interest, PollOpt::edge() | PollOpt::oneshot()).unwrap();
        }
        info!("WebSocketServer.ready(...): Done.");
    }
}

fn main() {
    LoggerFacade::init().unwrap();
    info!("Started...");
    let mut event_loop = EventLoop::new().unwrap();
    let mut server = WebSocketServer::new();

    event_loop.register_opt(&server.socket,  SERVER_TOKEN, EventSet::readable(),   PollOpt::edge()).unwrap();

    event_loop.run(&mut server).unwrap();
    info!("Done...");
}


