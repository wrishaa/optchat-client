package com.reframe.optchat.gpui.android;

import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.widget.EditText;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import android.util.Base64;

/** Device checks use actual GPUI hit testing and Android's EditText InputConnection. */
public final class OptChatDeviceChecks extends Instrumentation {
    private OptChatActivity activity;
    private File directory;
    private Bundle arguments;
    private int checks;
    private interface Check { boolean test() throws Exception; }

    @Override public void onCreate(Bundle args) { super.onCreate(args); arguments = args; start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            directory = getTargetContext().getFilesDir();
            if (!new File(directory, "test-mode").isFile()) throw new AssertionError("Enable app-private test-mode first");
            Intent launch = getTargetContext().getPackageManager().getLaunchIntentForPackage(getTargetContext().getPackageName());
            activity = (OptChatActivity)startActivitySync(launch);
            waitFor("OptChat phone layout", () -> state().has("phone"));
            // A new installation only: never modify existing user engines.
            if (state().optJSONArray("connected") != null && state().getJSONArray("connected").length() != 0)
                throw new AssertionError("Device suite requires an empty development app");
            secureStore();
            if (!probes().has("wsp.add")) tap("phone.avatar");
            tap("wsp.add");
            waitFor("Add engine form", () -> probes().has("wsp.token"));
            tap("wsp.token");
            waitFor("Android keyboard", () -> activity.getWindow().getDecorView().getRootWindowInsets()
                .isVisible(android.view.WindowInsets.Type.ime()));
            String endpoint = "http://127.0.0.1:" + arguments.getString("port", "18790");
            String header = new JSONObject().put("v", 1).put("urls", new JSONArray().put(endpoint)).put("id", "android-device-check").toString();
            String token = "oc1." + Base64.encodeToString(header.getBytes(StandardCharsets.UTF_8), Base64.NO_WRAP | Base64.NO_PADDING | Base64.URL_SAFE) + ".s3cret";
            edit(token, false);
            final InputConnection[] oldConnection = new InputConnection[1];
            runOnMainSync(() -> oldConnection[0] = editor().onCreateInputConnection(new EditorInfo()));
            tap("wsp.name"); edit("Android device check", false);
            final boolean[] staleRejected = new boolean[1];
            runOnMainSync(() -> staleRejected[0] = !oldConnection[0].commitText("stale-field-edit", 1));
            check(staleRejected[0], "Stale IME connection cannot edit a newly focused field");
            tap("wsp.add.go");
            waitFor("fake engine chat", () -> "Hello from the fake engine.".equals(state().optString("lastTalk")));
            waitFor("two workspaces on one socket", () -> state().getJSONArray("connected").length() == 2 && state().optInt("engineSockets") == 1);
            String prefs = new String(Files.readAllBytes(new File(directory, "prefs.json").toPath()), StandardCharsets.UTF_8);
            check(!prefs.contains(token), "Token absent from preferences");
            check(token.equals(activity.getToken("engine|" + endpoint)), "Engine token saved in Keystore-backed store");
            // Return to chat using the app's normal navigation before typing.
            activity.gpuiHideKeyboard(); SystemClock.sleep(500);
            tap("phone.back");
            tap("phone.chat");
            waitFor("phone composer", () -> probes().has("composer.input"));
            tap("composer.input");
            edit("Android café 🙂 বাংলা", true);
            waitFor("Unicode IME composition", () -> "Android café 🙂 বাংলা".equals(state().getJSONObject("phone").optString("draft")));
            runOnMainSync(() -> {
                InputConnection connection = editor().onCreateInputConnection(new EditorInfo());
                connection.setSelection(8, 12); connection.commitText("hello", 1); connection.finishComposingText();
            });
            waitFor("UTF-16 selection replacement", () -> "Android hello 🙂 বাংলা".equals(state().getJSONObject("phone").optString("draft")));
            tap("phone.chat");
            waitFor("chat send and response round trip", () -> "Android test reply received.".equals(state().optString("lastTalk")));
            waitFor("composer clears after sending", () -> "".equals(state().getJSONObject("phone").optString("draft")));
            // Let the shell script relaunch and verify saved connections independently.
            activity.gpuiHideKeyboard();
            result.putString("result", "PASS"); result.putInt("checks", checks);
            result.putString("stream", "PASS: " + checks + " Android device checks\n");
            finish(0, result);
        } catch (Throwable error) {
            result.putString("result", "FAIL"); result.putString("stream", error.toString() + "\n");
            finish(1, result);
        }
    }

    private EditText editor() { return activity.getWindow().getDecorView().findViewWithTag("OptChatIME"); }
    private void edit(String text, boolean composition) {
        runOnMainSync(() -> {
            InputConnection connection = editor().onCreateInputConnection(new EditorInfo());
            if (composition) { connection.setComposingText("Android café", 1); connection.setComposingText(text, 1); connection.finishComposingText(); }
            else connection.commitText(text, 1);
        });
        SystemClock.sleep(600);
    }
    private void secureStore() {
        String key = "android-device-check-only", token = "test-secret-never-a-real-token";
        check(activity.setToken(key, token), "Keystore encryption");
        check(token.equals(activity.getToken(key)), "Keystore decryption");
        String encrypted = getTargetContext().getSharedPreferences("encrypted_tokens", 0).getAll().toString();
        check(!encrypted.contains(token), "Token encrypted at rest");
        activity.deleteToken(key); check(activity.getToken(key) == null, "Token deletion");
    }
    private JSONObject state() throws Exception { return json("probes.json.state.json"); }
    private JSONObject probes() throws Exception { return json("probes.json"); }
    private JSONObject json(String name) throws Exception {
        File file = new File(directory, name); return file.isFile() ? new JSONObject(new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8)) : new JSONObject();
    }
    private void command(JSONObject value) throws Exception {
        Files.write(new File(directory, "probes.json.cmd").toPath(), new JSONArray().put(value).toString().getBytes(StandardCharsets.UTF_8));
        SystemClock.sleep(700);
    }
    private void waitFor(String label, Check condition) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 30000;
        while (SystemClock.uptimeMillis() < deadline) {
            try { if (condition.test()) { check(true, label); return; } } catch (Exception ignored) {}
            SystemClock.sleep(200);
        }
        throw new AssertionError("Timed out: " + label);
    }
    private void tap(String name) throws Exception {
        waitFor("Visible " + name, () -> probes().has(name));
        JSONArray bounds = probes().getJSONArray(name);
        float scale = activity.getResources().getDisplayMetrics().density;
        float x = ((float)bounds.getDouble(0) + (float)bounds.getDouble(2) / 2) * scale;
        float y = ((float)bounds.getDouble(1) + (float)bounds.getDouble(3) / 2) * scale;
        long now = SystemClock.uptimeMillis();
        MotionEvent down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, x, y, 0);
        MotionEvent up = MotionEvent.obtain(now, now + 80, MotionEvent.ACTION_UP, x, y, 0);
        down.setSource(InputDevice.SOURCE_TOUCHSCREEN); up.setSource(InputDevice.SOURCE_TOUCHSCREEN);
        getUiAutomation().injectInputEvent(down, true); getUiAutomation().injectInputEvent(up, true);
        down.recycle(); up.recycle(); SystemClock.sleep(700);
    }
    private void check(boolean ok, String label) { if (!ok) throw new AssertionError(label); checks++; }
}
