// c# host for the native optchat tv app: hands the process to the rust/gpui side (oc_main never returns while the app
// runs) and owns the mic: rust calls MicStart / MicStop on the remote's mic key down / up (through the shared
// dictation, crates/client). the remote's mic is not an audio device on the tv (AudioManager lists only the speaker,
// so AudioCapture records silence): like webapis.microphone (libwebapis_microphone.so, what the web tv shell uses) we
// ask the smart remote over bluetooth hid to send its voice (bt_hid_rc_start_sending_voice, extended every few
// seconds while held) and take the 16 kHz mono s16le it streams (bt_hid_set_audio_data_receive_cb), handing it back
// (oc_mic_pcm), then oc_mic_end once the tail after the release is in: the remote keeps sending what it buffered.
using System;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

static class Program {
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void Cb();
  [DllImport("libocgpui.so")] static extern int oc_main();
  [DllImport("libocgpui.so")] static extern void oc_set_mic(Cb start, Cb stop);
  [DllImport("libocgpui.so")] static extern void oc_mic_pcm(byte[] p, int n);
  [DllImport("libocgpui.so")] static extern void oc_mic_end();
  [DllImport("libocgpui.so")] static extern int oc_is_dev();

  // samsung tv bluetooth capi (signatures read off libwebapis_microphone.so's call sites)
  const string BT = "libcapi-network-bluetooth.so.0", BTV = "libcapi-network-bluetooth-tv.so.0", GLIB = "libglib-2.0.so.0";
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void ConnCb(int result, bool connected, IntPtr addr, IntPtr ud);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void AudioCb(IntPtr data, IntPtr ud); // data: {u16 len; ..; u8* buf @4}
  [DllImport(BT)] static extern int bt_initialize();
  [DllImport(BT)] static extern int bt_hid_host_initialize(ConnCb cb, IntPtr ud);
  [DllImport(BTV)] static extern int bt_hid_get_smart_rc_address(out IntPtr addr);
  [DllImport(BTV)] static extern int bt_hid_set_audio_data_receive_cb(AudioCb cb, IntPtr ud);
  [DllImport(BTV)] static extern int bt_hid_unset_audio_data_receive_cb();
  [DllImport(BTV)] static extern int bt_hid_rc_start_sending_voice(string addr);
  [DllImport(BTV)] static extern int bt_hid_rc_extend_sending_voice(string addr);
  [DllImport(BTV)] static extern int bt_hid_rc_stop_sending_voice(string addr);
  [DllImport(GLIB)] static extern IntPtr g_main_loop_new(IntPtr ctx, bool running);
  [DllImport(GLIB)] static extern void g_main_loop_run(IntPtr loop);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate bool GSourceFunc(IntPtr ud);
  [DllImport(GLIB)] static extern void g_main_context_invoke(IntPtr ctx, GSourceFunc f, IntPtr ud);
  // run f on the bt loop thread (bt's dbus subscriptions bind to the caller's context) and wait for it
  static T OnBt<T>(Func<T> f) {
    T r = default; Exception err = null; var done = new ManualResetEventSlim();
    GSourceFunc g = _ => { try { r = f(); } catch (Exception e) { err = e; } done.Set(); return false; };
    g_main_context_invoke(IntPtr.Zero, g, IntPtr.Zero);
    if (!done.Wait(5000)) throw new TimeoutException("bt loop busy");
    GC.KeepAlive(g); if (err != null) throw err; return r;
  }

  static readonly Cb startCb = MicStart, stopCb = MicStop; // kept alive for the native side
  static readonly ConnCb connCb = (r, c, a, u) => Beacon("bt rc conn " + r + " " + c);
  static readonly AudioCb audioCb = OnAudio;
  static readonly object gate = new object();
  static volatile bool held, recording; static bool dev, btReady; static string addr;
  static long total; static int peak, chunks; static DateTime lastData, lastLoud; static System.IO.MemoryStream dump;
  static Thread rec;
  const int TailMinMs = 800, TailQuietMs = 500, TailMaxMs = 4000, ExtendMs = 4000, Quiet = 300;

  // dev builds only: a log line to the mac log server
  static void Beacon(string m) {
    if (!dev) return;
    try { using (var c = new TcpClient("192.0.2.1", 8765)) { var b = Encoding.ASCII.GetBytes("GET /?m=" + Uri.EscapeDataString("host " + m) + " HTTP/1.0\r\nHost: x\r\n\r\n"); c.GetStream().Write(b, 0, b.Length); c.GetStream().Read(new byte[16], 0, 16); } } catch (Exception) { }
  }
  // dev builds only: the raw recording to the mac (nc-style sink on :8768) to check it by ear / stt
  static void Dump(byte[] b) {
    if (!dev) return;
    try { using (var c = new TcpClient("192.0.2.1", 8768)) { var h = Encoding.ASCII.GetBytes("btvoice-" + DateTime.Now.ToString("HHmmss") + ".raw\n"); c.GetStream().Write(h, 0, h.Length); c.GetStream().Write(b, 0, b.Length); } } catch (Exception) { }
  }

  // bluetooth callbacks come through glib's default main context, which nothing else runs here (gpui uses its own
  // loop): one thread initializes bt and runs that loop for the app's life
  static void BtLoop() {
    try {
      int r = bt_initialize(); int h = bt_hid_host_initialize(connCb, IntPtr.Zero);
      lock (gate) { btReady = true; Monitor.PulseAll(gate); }
      Beacon("bt init " + r + " hid " + h);
      g_main_loop_run(g_main_loop_new(IntPtr.Zero, false));
    } catch (Exception e) { Beacon("bt loop err " + e.GetType().Name + " " + e.Message); lock (gate) { btReady = true; Monitor.PulseAll(gate); } }
  }

  static void OnAudio(IntPtr data, IntPtr ud) {
    try {
      int n = (ushort)Marshal.ReadInt16(data); IntPtr p = Marshal.ReadIntPtr(data, 4);
      if (!recording || n <= 0 || p == IntPtr.Zero) return;
      var b = new byte[n]; Marshal.Copy(p, b, 0, n);
      int pk = 0; for (int i = 0; i + 1 < n; i += 2) { int v = Math.Abs((int)BitConverter.ToInt16(b, i)); if (v > pk) pk = v; }
      lock (gate) { total += n; chunks++; if (pk > peak) peak = pk; lastData = DateTime.Now; if (pk > Quiet) lastLoud = lastData; dump?.Write(b, 0, n); }
      oc_mic_pcm(b, n);
    } catch (Exception e) { Beacon("bt audio err " + e.Message); }
  }

  static void MicStart() {
    held = true;
    if (rec != null && rec.IsAlive) return;
    rec = new Thread(Record) { IsBackground = true }; rec.Start();
  }
  static void MicStop() { held = false; }
  static void Record() {
    lock (gate) { while (!btReady) Monitor.Wait(gate, 3000); total = 0; peak = 0; chunks = 0; lastData = lastLoud = DateTime.MinValue; dump = dev ? new System.IO.MemoryStream() : null; }
    bool started = false;
    try {
      if (addr == null) { IntPtr a = IntPtr.Zero; int g = OnBt(() => bt_hid_get_smart_rc_address(out a)); addr = a == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(a); Beacon("bt rc address " + g + " " + (addr == null ? "none" : addr.Length + " chars")); }
      if (addr == null) throw new Exception("no smart remote connected");
      int s = OnBt(() => bt_hid_set_audio_data_receive_cb(audioCb, IntPtr.Zero));
      recording = true;
      int st = OnBt(() => bt_hid_rc_start_sending_voice(addr)); started = st >= 0;
      Beacon("mic start cb " + s + " voice " + st);
      DateTime begun = DateTime.Now, extended = begun; DateTime? released = null;
      while (true) {
        Thread.Sleep(50);
        var now = DateTime.Now;
        if (!held && released == null) released = now;
        if (released != null) {
          DateTime ld, lq; lock (gate) { ld = lastData; lq = lastLoud; }
          double since = (now - released.Value).TotalMilliseconds;
          double still = (now - (ld > released.Value ? ld : released.Value)).TotalMilliseconds;
          double quiet = (now - (lq > released.Value ? lq : released.Value)).TotalMilliseconds;
          if (since > TailMaxMs || since > TailMinMs && (still > TailQuietMs || quiet > TailQuietMs)) break;
        } else if ((now - extended).TotalMilliseconds > ExtendMs) { extended = now; OnBt(() => bt_hid_rc_extend_sending_voice(addr)); }
      }
    } catch (Exception e) { Beacon("mic err " + e.GetType().Name + " " + e.Message); }
    try { if (started) Beacon("mic voice stop " + OnBt(() => bt_hid_rc_stop_sending_voice(addr))); } catch (Exception e) { Beacon("mic stop err " + e.Message); }
    recording = false;
    try { OnBt(() => bt_hid_unset_audio_data_receive_cb()); } catch (Exception) { }
    byte[] d; lock (gate) { d = dump?.ToArray(); dump = null; }
    Beacon("mic stop bytes " + total + " chunks " + chunks + " peak " + peak);
    oc_mic_end();
    if (d != null && d.Length > 0) new Thread(() => Dump(d)) { IsBackground = true }.Start();
  }
  static int Main(string[] args) {
    dev = oc_is_dev() != 0;
    new Thread(BtLoop) { IsBackground = true, Name = "bt" }.Start();
    oc_set_mic(startCb, stopCb); return oc_main();
  }
}
