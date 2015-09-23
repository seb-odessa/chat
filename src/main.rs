
extern crate mio;
extern crate syslog;

use std::net::SocketAddr;
use mio::tcp::*;
use mio::*;
use syslog::{Facility,Severity};
use std::collections::HashMap;

struct Logger {
    log : Box<syslog::Logger>
}
impl Logger {
    fn new() -> Logger{
        let log = match syslog::unix(Facility::LOG_USER) {
            Err(e)  => panic!("Impossible to connect to syslog: {:?}", e),
            Ok(log) => log
        };
        let logger = Logger { log : log };
        logger.info("Logger created successful.");
        return logger;
    }

    fn err<S: Into<String>>(&self, msg : S) {
        self.log.send_3164(Severity::LOG_ERR, msg.into()).unwrap(); 
    }

    fn info<S: Into<String>>(&self, msg : S) {
        self.log.send_3164(Severity::LOG_INFO, msg.into()).unwrap(); 
    }
}
impl Drop for Logger {
    fn drop(&mut self) {
        self.info("Logger destroyed.");
    }
}

const SERVER_TOKEN: Token = Token(1);
struct WebSocketServer {
    socket: TcpListener,
    clients: HashMap<Token, TcpStream>,
    token_counter: usize,
    log : Logger,
}
impl WebSocketServer {
    pub fn new() -> Self {
        let server = TcpSocket::v4().unwrap();
        let address = "0.0.0.0:10000".parse::<SocketAddr>().unwrap();
        server.bind(&address).unwrap();
        let server = server.listen(128).unwrap();
        WebSocketServer {
            socket : server,
            clients : HashMap::new(),
            token_counter : 1,
            log : Logger::new()
        }
    }
}
impl Handler for WebSocketServer {
    type Timeout = usize;
    type Message = ();
    fn ready(&mut self, event_loop: &mut EventLoop<WebSocketServer>, token: Token, events: EventSet)  {
        self.log.info("WebSocketServer.ready(...): Invoked.");
        match token {
            SERVER_TOKEN => {
                let client_socket = match self.socket.accept() {
                    Err(e) => {
                        self.log.err(format!("Accept error: {}", e));
                        return;
                    },
                    Ok(None) => unreachable!("Accept has returned 'None'"),
                    Ok(Some(sock)) => sock
                };
                
                self.token_counter += 1;
                let token = Token(self.token_counter);
                self.clients.insert(token, client_socket);
                event_loop.register_opt(&self.clients[&token], token, EventSet::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            },
            Token(_) => {
            }
        }
        self.log.info("WebSocketServer.ready(...): Done.");
    }
}

fn main() {
    let mut event_loop = EventLoop::new().unwrap();
    let mut server = WebSocketServer::new();

    event_loop.register_opt(&server.socket,  SERVER_TOKEN, EventSet::readable(),   PollOpt::edge()).unwrap();

    event_loop.run(&mut server).unwrap();
}


