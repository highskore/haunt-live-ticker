//! End to end against a real haunt core. Copy into haunt's `cli/tests/` and run
//! `cargo test -p haunt-cli --features plugins --test live_ticker` (see README).
#![cfg(feature = "plugins")]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use haunt_core::plugin_settings::PluginConfig;
use haunt_core::plugin_store::{Store, attach};
use haunt_core::plugins::PluginHost;
use haunt_core::{Core, SurfaceEnv};

const ID: &str = "dev.haunt.live-ticker";

fn repo() -> PathBuf {
    std::env::var_os("HAUNT_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../haunt-plugins"))
        .join("haunt-live-ticker")
}

fn until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(sock: &mut UnixStream, env: &SurfaceEnv, nonce: bool, cwd: &str, rest: &str) {
    let nonce = if nonce { format!(r#","nonce":"{}""#, env.nonce) } else { String::new() };
    let line = format!(r#"{{"v":1,"agent":"claude","surface_id":"{}"{nonce},"session_id":"s1","cwd":"{cwd}",{rest}}}"#, env.surface_id);
    sock.write_all(format!("{line}\n").as_bytes()).unwrap();
}

#[test]
fn the_meta_line_follows_the_agents_calls() {
    let dir = repo();
    if !dir.join("haunt-plugin.toml").is_file() {
        eprintln!("skipped: haunt-live-ticker not found; set HAUNT_PLUGINS_DIR");
        return;
    }
    let home = PathBuf::from(format!("/tmp/haunt-fp-ticker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&dir)
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .unwrap();
    assert!(status.success());
    let o = Command::new(env!("CARGO_BIN_EXE_haunt")).args(["plugin", "install", dir.to_str().unwrap(), "--yes"]).env("HAUNT_HOME", &home).output().unwrap();
    assert!(o.status.success(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));

    let core = Core::new(&home);
    core.start().unwrap();
    let (env, other) = (core.register_surface("/tmp").unwrap(), core.register_surface("/tmp").unwrap());
    let set = Store::open(&home).unwrap().load_all(&PluginHost::new().unwrap(), Some(&core.handle())).unwrap();
    assert!(set.errors().is_empty(), "{:?}", set.errors());
    let set = Arc::new(Mutex::new(set));
    attach(&core, &set);
    let suffix = |s: &str| core.snapshot().groups.iter().flat_map(|g| &g.rows).find(|r| r.surface_id == s).and_then(|r| r.decoration.clone()).and_then(|d| d.suffix);

    let mut sock = UnixStream::connect(&env.socket_path).unwrap();
    send(&mut sock, &env, true, "/tmp/web", r#""event":"prompt_submit","query":"go""#);
    send(&mut sock, &env, true, "/tmp/web", r#""event":"tool_start","tool_name":"Read","tool_use_id":"t1","tool_input":{"file_path":"/tmp/web/session.ts"}"#);
    send(&mut sock, &env, true, "/tmp/web", r#""event":"tool_start","tool_name":"Bash","tool_use_id":"t2","tool_input":{"command":"npm test"}"#);
    send(&mut sock, &other, false, "/tmp/web", r#""event":"tool_start","tool_name":"Bash","tool_use_id":"t3","tool_input":{"command":"curl evil"}"#);
    until("the ticker", || suffix(&env.surface_id).as_deref() == Some("Read session.ts → Bash npm test →"));
    let d = core.snapshot().groups.iter().flat_map(|g| &g.rows).find(|r| r.surface_id == env.surface_id).unwrap().decoration.clone().unwrap();
    assert!(d.scroll);
    assert_eq!(suffix(&other.surface_id), None, "forged calls show nothing");

    send(&mut sock, &env, true, "/tmp/web", r#""event":"permission_seen","tool_name":"Bash""#);
    until("haunt's status to own the needs-you row", || suffix(&env.surface_id).is_none());
    send(&mut sock, &env, true, "/tmp/web", r#""event":"stop""#);
    until("the ticker to clear", || suffix(&env.surface_id).is_none());

    send(&mut sock, &env, true, "/tmp/web", r#""event":"tool_start","tool_name":"Grep","tool_use_id":"t4","tool_input":{"pattern":"x"}"#);
    until("a new call", || suffix(&env.surface_id).as_deref() == Some("Grep →"));
    set.lock().unwrap().set_config(&PluginConfig::parse(&format!("[plugins.\"{ID}\"]\nscroll = false\n")));
    let d = core.snapshot().groups.iter().flat_map(|g| &g.rows).find(|r| r.surface_id == env.surface_id).unwrap().decoration.clone().unwrap();
    assert!(!d.scroll);

    drop(core);
    let _ = std::fs::remove_dir_all(&home);
}
