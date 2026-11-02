// c# host for the native optchat tv app: hands the process to the rust/gpui side (oc_main never returns while the app
// runs) and owns the mic: rust calls MicStart / MicStop on the remote's mic key down / up (through the shared
// dictation, crates/client); we record with Tizen.Multimedia AudioCapture (16 kHz mono s16le, voice-recognition
// policy) and hand the pcm back (oc_mic_pcm), then oc_mic_end once the tail after the release is in: the remote
// keeps sending what it buffered for a moment after the key comes up.
using System;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Tizen.Multimedia;

static class Program {
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void Cb();
  [DllImport("libocgpui.so")] static extern int oc_main();
  [DllImport("libocgpui.so")] static extern void oc_set_mic(Cb start, Cb stop);
  [DllImport("libocgpui.so")] static extern void oc_mic_pcm(byte[] p, int n);
  [DllImport("libocgpui.so")] static extern void oc_mic_end();
  [DllImport("libocgpui.so")] static extern int oc_is_dev();
  static readonly Cb startCb = MicStart, stopCb = MicStop; // kept alive for the native side
  static volatile bool held; static Thread rec; static bool dev;
  const int TailMinMs = 800, TailQuietMs = 400, TailMaxMs = 4000, Quiet = 300;

  // dev builds only: a log line to the mac log server
  static void Beacon(string m) {
    if (!dev) return;
    try { using (var c = new TcpClient("192.0.2.1", 8765)) { var b = Encoding.ASCII.GetBytes("GET /?m=" + Uri.EscapeDataString("host " + m) + " HTTP/1.0\r\nHost: x\r\n\r\n"); c.GetStream().Write(b, 0, b.Length); c.GetStream().Read(new byte[16], 0, 16); } } catch (Exception) { }
  }
  static void MicStart() {
    held = true;
    if (rec != null && rec.IsAlive) return;
    rec = new Thread(Record) { IsBackground = true }; rec.Start();
  }
  static void MicStop() { held = false; }
  static void Record() {
    long total = 0; int peak = 0;
    try {
      using (var cap = new AudioCapture(16000, AudioChannel.Mono, AudioSampleType.S16Le)) {
        try { cap.ApplyStreamPolicy(new AudioStreamPolicy(AudioStreamType.VoiceRecognition)); } catch (Exception e) { Beacon("mic policy " + e.GetType().Name + " " + e.Message); }
        cap.Prepare();
        int sz = cap.GetBufferSize(); Beacon("mic start buf " + sz);
        DateTime? released = null, lastLoud = null;
        while (true) {
          var now = DateTime.Now;
          if (!held && released == null) released = now;
          if (released != null) {
            double since = (now - released.Value).TotalMilliseconds, quiet = (now - (lastLoud ?? released.Value)).TotalMilliseconds;
            if (since > TailMaxMs || since > TailMinMs && quiet > TailQuietMs) break;
          }
          var b = cap.Read(sz); total += b.Length;
          int p = 0; for (int i = 0; i + 1 < b.Length; i += 2) { int v = Math.Abs((int)BitConverter.ToInt16(b, i)); if (v > p) p = v; }
          if (p > peak) peak = p;
          if (p > Quiet) lastLoud = DateTime.Now;
          oc_mic_pcm(b, b.Length);
        }
        cap.Unprepare();
      }
    } catch (Exception e) { Beacon("mic err " + e.GetType().Name + " " + e.Message); }
    Beacon("mic stop bytes " + total + " peak " + peak);
    oc_mic_end();
  }
  static int Main(string[] args) { dev = oc_is_dev() != 0; oc_set_mic(startCb, stopCb); return oc_main(); }
}
