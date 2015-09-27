
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



struct HttpParser;
impl ParserHandler for HttpParser { }


const SERVER_TOKEN: Token = Token(1);

struct WebSocketClient{
    socket: TcpStream,
    http_parser: Parser<HttpParser>,
}

impl WebSocketClient{
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
                        // ...
                        break;
                    }
                }
            }
        }
    }

    fn new(socket: TcpStream) -> WebSocketClient {
        WebSocketClient {
            socket: socket,
            http_parser: Parser::request(HttpParser),
        }
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

    fn ready(&mut self, event_loop: &mut EventLoop<WebSocketServer>, token: Token, _events: EventSet)  {
        info!("WebSocketServer.ready(...): Invoked.");
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
                event_loop.reregister(&client.socket, token, EventSet::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            },          
        }
        info!("WebSocketServer.ready(...): Done.");
    }
}

fn main() {
    LoggerFacade::init().unwrap();
    info!("Started...{},{}",1,2);
    warn!("Warning");
    error!("Error!!!");
    let mut event_loop = EventLoop::new().unwrap();
    let mut server = WebSocketServer::new();

    event_loop.register_opt(&server.socket,  SERVER_TOKEN, EventSet::readable(),   PollOpt::edge()).unwrap();

    event_loop.run(&mut server).unwrap();
    info!("Done...");
}


