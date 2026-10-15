//! Apple (macOS, iOS): bonjour browsing through dns_sd (libSystem: DNSServiceBrowse → Resolve → GetAddrInfo) and
//! network path changes through Network.framework's NWPathMonitor. Shells that want lan-first call these from their
//! `Platform::browse` / `Platform::watch_net`.
//!
//! iOS: browsing needs `NSBonjourServices` (the service type) and `NSLocalNetworkUsageDescription` in Info.plist; the
//! first browse or lan connect shows the system's "Local Network" prompt. Denied → no results (the tunnel stays).
use crate::{Bonjour, BonjourEvent, NetPath};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type DNSServiceRef = *mut c_void;
type Flags = u32;
type Err = i32;
const FLAG_ADD: u32 = 0x2;
const PROTO_V4: u32 = 0x1;

type BrowseReply = extern "C" fn(DNSServiceRef, Flags, u32, Err, *const c_char, *const c_char, *const c_char, *mut c_void);
type ResolveReply = extern "C" fn(DNSServiceRef, Flags, u32, Err, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
type AddrReply = extern "C" fn(DNSServiceRef, Flags, u32, Err, *const c_char, *const SockAddr, u32, *mut c_void);

#[repr(C)]
pub struct SockAddr {
    sa_len: u8,
    sa_family: u8,
    sa_data: [u8; 14],
}

unsafe extern "C" {
    fn DNSServiceBrowse(r: *mut DNSServiceRef, f: Flags, ifi: u32, regtype: *const c_char, domain: *const c_char, cb: BrowseReply, ctx: *mut c_void) -> Err;
    fn DNSServiceResolve(r: *mut DNSServiceRef, f: Flags, ifi: u32, name: *const c_char, regtype: *const c_char, domain: *const c_char, cb: ResolveReply, ctx: *mut c_void) -> Err;
    fn DNSServiceGetAddrInfo(r: *mut DNSServiceRef, f: Flags, ifi: u32, proto: u32, host: *const c_char, cb: AddrReply, ctx: *mut c_void) -> Err;
    fn DNSServiceRefSockFD(r: DNSServiceRef) -> i32;
    fn DNSServiceProcessResult(r: DNSServiceRef) -> Err;
    fn DNSServiceRefDeallocate(r: DNSServiceRef);
    fn poll(fds: *mut PollFd, n: u32, timeout: i32) -> i32;
}
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}
const POLLIN: i16 = 0x1;

/// one instance being resolved / followed
struct Inst {
    svc: Bonjour,
    resolve: DNSServiceRef,
    addr: DNSServiceRef,
}
/// everything the dns_sd thread owns; callbacks get a pointer to it (only that thread touches it)
struct St {
    regtype: CString,
    on: Arc<dyn Fn(BonjourEvent) + Send + Sync>,
    browse: DNSServiceRef,
    insts: HashMap<String, Inst>,
    /// refs to deallocate after DNSServiceProcessResult returns (never inside their own callback)
    dead: Vec<DNSServiceRef>,
}

extern "C" fn on_browse(_r: DNSServiceRef, flags: Flags, ifi: u32, err: Err, name: *const c_char, regtype: *const c_char, domain: *const c_char, ctx: *mut c_void) {
    let st = unsafe { &mut *(ctx as *mut St) };
    if err != 0 || name.is_null() {
        return;
    }
    let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    if flags & FLAG_ADD != 0 {
        if st.insts.contains_key(&n) {
            return;
        }
        let mut r: DNSServiceRef = std::ptr::null_mut();
        let ok = unsafe { DNSServiceResolve(&mut r, 0, ifi, name, regtype, domain, on_resolve, ctx) } == 0;
        if ok {
            st.insts.insert(n.clone(), Inst { svc: Bonjour { name: n, ..Default::default() }, resolve: r, addr: std::ptr::null_mut() });
        }
    } else if let Some(i) = st.insts.remove(&n) {
        for r in [i.resolve, i.addr] {
            if !r.is_null() {
                st.dead.push(r);
            }
        }
        (st.on)(BonjourEvent::Lost(n));
    }
}

fn parse_txt(b: &[u8]) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let l = b[i] as usize;
        let e = (i + 1 + l).min(b.len());
        let kv = String::from_utf8_lossy(&b[i + 1..e]).into_owned();
        if !kv.is_empty() {
            match kv.split_once('=') {
                Some((k, v)) => out.push((k.to_string(), v.to_string())),
                None => out.push((kv, String::new())),
            }
        }
        i = e;
    }
    out
}

extern "C" fn on_resolve(r: DNSServiceRef, _f: Flags, ifi: u32, err: Err, _full: *const c_char, host: *const c_char, port_be: u16, txt_len: u16, txt: *const u8, ctx: *mut c_void) {
    let st = unsafe { &mut *(ctx as *mut St) };
    let Some((_, inst)) = st.insts.iter_mut().find(|(_, i)| i.resolve == r) else { return };
    st.dead.push(r);
    inst.resolve = std::ptr::null_mut();
    if err != 0 || host.is_null() {
        return;
    }
    inst.svc.host = unsafe { CStr::from_ptr(host) }.to_string_lossy().trim_end_matches('.').to_string();
    inst.svc.port = u16::from_be(port_be);
    inst.svc.txt = if txt.is_null() { vec![] } else { parse_txt(unsafe { std::slice::from_raw_parts(txt, txt_len as usize) }) };
    let mut a: DNSServiceRef = std::ptr::null_mut();
    if unsafe { DNSServiceGetAddrInfo(&mut a, 0, ifi, PROTO_V4, host, on_addr, ctx) } == 0 {
        inst.addr = a;
    }
}

extern "C" fn on_addr(r: DNSServiceRef, flags: Flags, _ifi: u32, err: Err, _host: *const c_char, sa: *const SockAddr, _ttl: u32, ctx: *mut c_void) {
    let st = unsafe { &mut *(ctx as *mut St) };
    let on = st.on.clone();
    let Some((_, inst)) = st.insts.iter_mut().find(|(_, i)| i.addr == r) else { return };
    if err != 0 || sa.is_null() {
        return;
    }
    let s = unsafe { &*sa };
    if s.sa_family != 2 {
        return; // AF_INET only
    }
    // sockaddr_in: port (2 bytes) then the address (4 bytes)
    let ip = format!("{}.{}.{}.{}", s.sa_data[2], s.sa_data[3], s.sa_data[4], s.sa_data[5]);
    let had = inst.svc.addrs.contains(&ip);
    if flags & FLAG_ADD != 0 && !had {
        inst.svc.addrs.push(ip);
    } else if flags & FLAG_ADD == 0 && had {
        inst.svc.addrs.retain(|a| a != &ip);
    } else {
        return;
    }
    on(BonjourEvent::Found(inst.svc.clone()));
}

/// The running browse: dropping it stops the thread and frees every dns_sd ref.
pub struct BrowseHandle {
    stop: Arc<AtomicBool>,
}
impl Drop for BrowseHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Browse `regtype` (e.g. `_optchat-lan._tcp`) in `local.`: Found for every resolved instance (again when its
/// addresses change), Lost when it goes away.
pub fn browse(regtype: &str, on: Arc<dyn Fn(BonjourEvent) + Send + Sync>) -> Option<BrowseHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let regtype = CString::new(regtype).ok()?;
    let s2 = stop.clone();
    let (ok_tx, ok_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("bonjour".into())
        .spawn(move || {
            let st = Box::into_raw(Box::new(St { regtype, on, browse: std::ptr::null_mut(), insts: HashMap::new(), dead: vec![] }));
            let dom = c"local.";
            let mut r: DNSServiceRef = std::ptr::null_mut();
            let e = unsafe { DNSServiceBrowse(&mut r, 0, 0, (*st).regtype.as_ptr(), dom.as_ptr(), on_browse, st as *mut c_void) };
            let _ = ok_tx.send(e == 0);
            if e != 0 {
                drop(unsafe { Box::from_raw(st) });
                return;
            }
            unsafe { (*st).browse = r };
            while !s2.load(Ordering::SeqCst) {
                let refs: Vec<DNSServiceRef> = unsafe {
                    let s = &*st;
                    std::iter::once(s.browse).chain(s.insts.values().flat_map(|i| [i.resolve, i.addr])).filter(|r| !r.is_null()).collect()
                };
                let mut fds: Vec<PollFd> = refs.iter().map(|r| PollFd { fd: unsafe { DNSServiceRefSockFD(*r) }, events: POLLIN, revents: 0 }).collect();
                let n = unsafe { poll(fds.as_mut_ptr(), fds.len() as u32, 250) };
                if n > 0 {
                    for (i, f) in fds.iter().enumerate() {
                        if f.revents != 0 {
                            // a ref freed by an earlier callback in this round is skipped
                            if unsafe { (*st).dead.contains(&refs[i]) } {
                                continue;
                            }
                            if unsafe { DNSServiceProcessResult(refs[i]) } != 0 && refs[i] == unsafe { (*st).browse } {
                                // mDNSResponder went away: stop (the client restarts the browse on the next path change)
                                s2.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }
                for d in unsafe { std::mem::take(&mut (*st).dead) } {
                    unsafe { DNSServiceRefDeallocate(d) };
                }
            }
            let s = unsafe { Box::from_raw(st) };
            for i in s.insts.values() {
                for r in [i.resolve, i.addr] {
                    if !r.is_null() {
                        unsafe { DNSServiceRefDeallocate(r) };
                    }
                }
            }
            unsafe { DNSServiceRefDeallocate(s.browse) };
        })
        .ok()?;
    ok_rx.recv().ok()?.then_some(BrowseHandle { stop })
}

// ---- NWPathMonitor ---------------------------------------------------------------------------------------------------
#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_path_monitor_create() -> *mut c_void;
    fn nw_path_monitor_set_update_handler(m: *mut c_void, h: &block2::Block<dyn Fn(*mut c_void)>);
    fn nw_path_monitor_set_queue(m: *mut c_void, q: *mut c_void);
    fn nw_path_monitor_start(m: *mut c_void);
    fn nw_path_get_status(p: *mut c_void) -> i32;
    fn nw_path_uses_interface_type(p: *mut c_void, t: i32) -> bool;
    fn nw_path_is_expensive(p: *mut c_void) -> bool;
}
unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
}

/// Watch the network path for the life of the process: `on` gets the path at once and after every change.
pub fn watch_net(on: Arc<dyn Fn(NetPath) + Send + Sync>) -> bool {
    unsafe {
        let m = nw_path_monitor_create();
        if m.is_null() {
            return false;
        }
        let block = block2::RcBlock::new(move |p: *mut c_void| {
            let np = NetPath {
                up: nw_path_get_status(p) == 1,
                wifi: nw_path_uses_interface_type(p, 1),
                cellular: nw_path_uses_interface_type(p, 2),
                wired: nw_path_uses_interface_type(p, 3),
                expensive: nw_path_is_expensive(p),
            };
            on(np);
        });
        nw_path_monitor_set_update_handler(m, &block);
        let q = dispatch_queue_create(c"optchat.netpath".as_ptr(), std::ptr::null_mut());
        nw_path_monitor_set_queue(m, q);
        nw_path_monitor_start(m);
        // the monitor and its handler live as long as the app
        std::mem::forget(block);
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn txt_records() {
        let raw = b"\x03v=1\x0aengine=abc\x00\x04flag";
        assert_eq!(super::parse_txt(raw), vec![("v".into(), "1".into()), ("engine".into(), "abc".into()), ("flag".into(), String::new())]);
    }
}

/// live checks against the Mac's mDNSResponder (`cargo test -p optchat-platform --features apple-net -- --ignored`)
#[cfg(test)]
mod live {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    #[test]
    #[ignore]
    fn browse_finds_and_loses_a_service() {
        let mut reg = std::process::Command::new("dns-sd").args(["-R", "OptChat LanTest", "_optchat-lantest._tcp", "local", "18999", "v=1", "engine=e1", "pin=sha256/AAAA"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        let seen: Arc<Mutex<Vec<BonjourEvent>>> = Arc::default();
        let s2 = seen.clone();
        let t0 = Instant::now();
        let h = browse("_optchat-lantest._tcp", Arc::new(move |e| s2.lock().unwrap().push(e))).expect("browse");
        let wait = |f: &dyn Fn(&[BonjourEvent]) -> bool| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(5) {
                if f(&seen.lock().unwrap()) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        };
        assert!(wait(&|e| e.iter().any(|e| matches!(e, BonjourEvent::Found(b) if b.port == 18999 && !b.addrs.is_empty()))), "found: {:?}", seen.lock().unwrap());
        eprintln!("found in {:?}: {:?}", t0.elapsed(), seen.lock().unwrap().last());
        let found = seen.lock().unwrap().iter().find_map(|e| if let BonjourEvent::Found(b) = e { Some(b.clone()) } else { None }).unwrap();
        assert!(found.txt.contains(&("engine".into(), "e1".into())));
        let t1 = Instant::now();
        reg.kill().unwrap();
        assert!(wait(&|e| e.iter().any(|e| matches!(e, BonjourEvent::Lost(n) if n == "OptChat LanTest"))));
        eprintln!("lost in {:?}", t1.elapsed());
        drop(h);
        let net: Arc<Mutex<Option<NetPath>>> = Arc::default();
        let n2 = net.clone();
        assert!(watch_net(Arc::new(move |p| *n2.lock().unwrap() = Some(p))));
        std::thread::sleep(Duration::from_millis(300));
        eprintln!("path: {:?}", net.lock().unwrap());
        assert!(net.lock().unwrap().is_some_and(|p| p.up));
    }
}
