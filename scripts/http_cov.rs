// Copied into the mcp-server-rs checkout by scripts/coverage.sh (as
// examples/http_cov.rs). Runs the library's HTTP transport on an ephemeral
// port, prints the address, and exits through `process::exit` when stdin
// closes, so the coverage runtime writes its profile.
use mcp_server::transport::http::{HttpConfig, HttpServer};
use mcp_server::Server;
use std::io::Read;

fn main() {
    let server = HttpServer::bind("127.0.0.1:0", Server::new(), HttpConfig::default())
        .expect("bind");
    println!("http://{}/mcp", server.local_addr());
    let _workers = server.spawn();
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    std::process::exit(0);
}
