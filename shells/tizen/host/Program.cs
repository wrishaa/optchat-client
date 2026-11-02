// c# host for the native optchat tv shell: hand the process to the rust/gpui side (oc_main never returns while the app runs).
using System.Runtime.InteropServices;

static class Program {
  [DllImport("libocgpui.so")] static extern int oc_main();
  static int Main(string[] args) { return oc_main(); }
}
