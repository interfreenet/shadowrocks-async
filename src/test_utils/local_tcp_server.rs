use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::info;

use crate::Result;

// Run a single-threaded TCP server that reads two bytes and writes four bytes.
// The returned bytes are either [12, 09] + counter, or [00, 02] + counter.
pub fn run_local_tcp_server() -> Result<(SocketAddr, Arc<AtomicBool>)> {
    let tcp_listener = TcpListener::bind("127.0.0.1:0")?;
    let local_addr = tcp_listener.local_addr()?;
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    std::thread::spawn(move || {
        let mut count = 0u16;
        #[allow(clippy::explicit_counter_loop)]
        for stream in tcp_listener.incoming() {
            count += 1;
            match stream {
                Ok(mut stream) => {
                    info!(
                        "Accepted connection {} from {}",
                        count,
                        stream.peer_addr()?
                    );
                    let mut buf = [0u8; 2];
                    stream.read_exact(&mut buf)?;

                    let mask = 1209u16.to_be_bytes();
                    let crv = u16::from_be_bytes(buf);
                    let prefix_bytes: &[u8] =
                        if crv == 1990 { &mask } else { &[0x00, 0x02] };
                    stream.write_all(
                        &[prefix_bytes, &count.to_be_bytes()].concat(),
                    )?;
                    stream.flush()?;
                }
                Err(e) => return Err(e),
            }
            if !running.load(Ordering::Relaxed) {
                break;
            }
        }
        Ok(())
    });

    Ok((local_addr, running_clone))
}
