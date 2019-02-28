extern crate mioco;

use std::net::SocketAddr;
use std::str::FromStr;
use std::io::{self, Read, Write};
use mioco::net::TcpListener;

const DEFAULT_LISTEN_ADDR : &'static str = "127.0.0.1:5555";

fn listend_addr() -> SocketAddr {
    FromStr::from_str(DEFAULT_LISTEN_ADDR).unwrap()
}

macro_rules! printerrln {
    ($($arg:tt)*) => ({
        use std::io::prelude::*;
        if let Err(e) = writeln!(&mut ::std::io::stderr(), "{}",
            format_args!($($arg)*)) {
            panic!(concat!(
                    "Failed to write to stderr.\n",
                    "Original error output: {}\n",
                    "Secondary error writing to stderr: {}"),
                    format_args!($($arg)*), e);
        }
    })
}

fn main() {
    let _join = mioco::spawn(|| -> io::Result<()> {
        let addr = listend_addr();

        let listener = try!(TcpListener::bind(&addr));

        printerrln!("Starting tcp echo server on {:?}", try!(listener.local_addr()));

        loop {
            let (mut conn, _addr) = try!(listener.accept());

            let _join = mioco::spawn(move || -> io::Result<()> {
                let mut buf = vec![0u8; 1024 * 64];
                loop {
                    let size = try!(conn.read(&mut buf));
                    if size == 0 {/* eof */ break; }
                    let _ = try!(conn.write_all(&mut buf[0..size]));
                }

                Ok(())
            });
        }
    });

    std::thread::sleep(std::time::Duration::from_micros(10_000_000))
}
