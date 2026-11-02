// c# host for the native optchat tv shell: hand the process to the rust/gpui side (oc_main never returns while the app runs).
// also owns the mic: rust calls MicStart/MicStop (remote mic button down/up); we record with Tizen.Multimedia
// AudioCapture (16 kHz mono s16le, voice-recognition stream policy) and, for this poc, stream pcm to the mac sink.
using System;
using System.IO;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Tizen.Multimedia;

static class Program {
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void Cb();
  [DllImport("libocgpui.so")] static extern int oc_main();
  [DllImport("libocgpui.so")] static extern void oc_set_mic(Cb start, Cb stop);
  static readonly Cb startCb = MicStart, stopCb = MicStop; // kept alive for the native side
  const string Mac = "192.0.2.1"; // poc dev sinks: :8765 log beacon, :8766 pcm sink
  static volatile bool held; static Thread rec;

  static void Beacon(string m) {
    try { using (var c = new TcpClient(Mac, 8765)) { var b = Encoding.ASCII.GetBytes("GET /?m=" + Uri.EscapeDataString("host " + m) + " HTTP/1.0\r\nHost: x\r\n\r\n"); c.GetStream().Write(b, 0, b.Length); c.GetStream().Read(new byte[16], 0, 16); } } catch (Exception) { }
  }
  static void Post(string tag, byte[] d, int n) {
    try { using (var c = new TcpClient(Mac, 8766)) { var s = c.GetStream(); var h = Encoding.ASCII.GetBytes("POST /" + tag + " HTTP/1.0\r\nHost: x\r\nContent-Length: " + n + "\r\n\r\n"); s.Write(h, 0, h.Length); s.Write(d, 0, n); s.Read(new byte[16], 0, 16); } } catch (Exception e) { Beacon("post err " + e.Message); }
  }
  static void MicStart() {
    held = true;
    if (rec != null && rec.IsAlive) return;
    rec = new Thread(Record) { IsBackground = true }; rec.Start();
  }
  static void MicStop() { held = false; }
  static void Record() {
    var tag = "n" + DateTime.Now.ToString("HHmmss");
    try {
      using (var cap = new AudioCapture(16000, AudioChannel.Mono, AudioSampleType.S16Le)) {
        try { cap.ApplyStreamPolicy(new AudioStreamPolicy(AudioStreamType.VoiceRecognition)); } catch (Exception e) { Beacon("mic policy " + e.GetType().Name + " " + e.Message); }
        cap.Prepare();
        int sz = cap.GetBufferSize(); Beacon("mic start " + tag + " buf " + sz);
        var acc = new MemoryStream(); long total = 0; int peak = 0; DateTime? released = null;
        // keep recording ~600 ms after release so the end of the sentence isn't cut (the remote buffers)
        while (true) {
          if (!held && released == null) released = DateTime.Now;
          if (released != null && (DateTime.Now - released.Value).TotalMilliseconds > 600) break;
          var b = cap.Read(sz); acc.Write(b, 0, b.Length); total += b.Length;
          for (int i = 0; i + 1 < b.Length; i += 2) { int v = Math.Abs((int)BitConverter.ToInt16(b, i)); if (v > peak) peak = v; }
          if (acc.Length >= 16000) { Post(tag, acc.GetBuffer(), (int)acc.Length); acc.SetLength(0); }
        }
        if (acc.Length > 0) Post(tag, acc.GetBuffer(), (int)acc.Length);
        cap.Unprepare();
        Beacon("mic stop " + tag + " bytes " + total + " peak " + peak);
      }
    } catch (Exception e) { Beacon("mic err " + e.GetType().Name + " " + e.Message); }
  }
  static int Main(string[] args) { oc_set_mic(startCb, stopCb); return oc_main(); }
}
