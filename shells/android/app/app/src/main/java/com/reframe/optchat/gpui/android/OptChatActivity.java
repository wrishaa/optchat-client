package com.reframe.optchat.gpui.android;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Bundle;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.util.Base64;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.MessageDigest;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** Native GPUI owns rendering. This activity supplies Android text input and OS services. */
public final class OptChatActivity extends NativeActivity {
    static { System.loadLibrary("optchat_android"); }
    private Editor editor;
    private boolean applyingNative;
    private long session;
    private long revision;
    private static final String KEY_ALIAS = "optchat.client.tokens.v1";

    private native void nativeEditor(long session, long revision, String text,
        int start, int end, int composingStart, int composingEnd);
    private native void nativeInsets(int top, int right, int bottom, int left);
    private native void nativeBack();

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        editor = new Editor();
        editor.setTag("OptChatIME");
        editor.setSingleLine(false);
        // Tokens must never be offered to keyboard personalization or autofill services.
        editor.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE
            | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        editor.setImeOptions(EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        editor.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
        editor.setAlpha(0f);
        addContentView(editor, new ViewGroup.LayoutParams(1, 1));
        View decor = getWindow().getDecorView();
        decor.setOnApplyWindowInsetsListener((view, insets) -> {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                android.graphics.Insets ime = insets.getInsets(WindowInsets.Type.ime());
                nativeInsets(bars.top, bars.right, Math.max(bars.bottom, ime.bottom), bars.left);
            } else {
                nativeInsets(insets.getSystemWindowInsetTop(), insets.getSystemWindowInsetRight(),
                    insets.getSystemWindowInsetBottom(), insets.getSystemWindowInsetLeft());
            }
            return view.onApplyWindowInsets(insets);
        });
        decor.requestApplyInsets();
        if (android.os.Build.VERSION.SDK_INT >= 33)
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, this::nativeBack);
    }

    // API 26–32 fallback; API 33+ gestures use the OnBackInvokedDispatcher registered above.
    @android.annotation.SuppressLint("GestureBackNavigation")
    @Override public void onBackPressed() { nativeBack(); }
    public void gpuiBackground() { runOnUiThread(() -> moveTaskToBack(true)); }
    public void gpuiAppearance(boolean dark) {
        runOnUiThread(() -> {
            getWindow().setStatusBarColor(android.graphics.Color.TRANSPARENT);
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                int mask = android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                    | android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                getWindow().getInsetsController().setSystemBarsAppearance(dark ? 0 : mask, mask);
            } else {
                View decor = getWindow().getDecorView();
                int mask = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                decor.setSystemUiVisibility((decor.getSystemUiVisibility() & ~mask) | (dark ? 0 : mask));
            }
        });
    }

    public void gpuiShowKeyboard() {
        runOnUiThread(() -> {
            editor.requestFocus();
            ((InputMethodManager)getSystemService(INPUT_METHOD_SERVICE)).showSoftInput(editor, InputMethodManager.SHOW_IMPLICIT);
        });
    }
    public void gpuiHideKeyboard() {
        runOnUiThread(() -> ((InputMethodManager)getSystemService(INPUT_METHOD_SERVICE))
            .hideSoftInputFromWindow(editor.getWindowToken(), 0));
    }

    /** Native sends acknowledgements so a delayed repaint cannot erase newer IME edits. */
    public void gpuiUpdateEditor(long field, long acknowledged, String text, int start, int end) {
        runOnUiThread(() -> {
            if (field == session && acknowledged < revision) return;
            boolean changedField = session != field;
            applyingNative = true;
            try {
                session = field;
                if (changedField) { revision = 0; editor.batchDepth = 0; }
                if (!editor.getText().toString().equals(text)) editor.setText(text);
                int length = editor.length();
                Selection.setSelection(editor.getText(), Math.max(0, Math.min(start, length)), Math.max(0, Math.min(end, length)));
            } finally { applyingNative = false; }
            if (changedField && editor.hasFocus())
                ((InputMethodManager)getSystemService(INPUT_METHOD_SERVICE)).restartInput(editor);
        });
    }

    private void publishEditor() {
        if (applyingNative || session == 0 || editor == null) return;
        Editable value = editor.getText();
        nativeEditor(session, ++revision, value.toString(), editor.getSelectionStart(), editor.getSelectionEnd(),
            BaseInputConnection.getComposingSpanStart(value), BaseInputConnection.getComposingSpanEnd(value));
    }

    private final class Editor extends EditText {
        private int batchDepth;
        Editor() { super(OptChatActivity.this); }
        @Override public InputConnection onCreateInputConnection(EditorInfo info) {
            InputConnection target = super.onCreateInputConnection(info);
            final long fieldSession = session;
            return new InputConnectionWrapper(target, false) {
                private boolean active() { return fieldSession == session; }
                private boolean done(boolean result) { if (batchDepth == 0) publishEditor(); return result; }
                @Override public boolean beginBatchEdit() { if (!active()) return false; batchDepth++; return super.beginBatchEdit(); }
                @Override public boolean endBatchEdit() {
                    if (!active()) return false;
                    boolean result = super.endBatchEdit(); batchDepth = Math.max(0, batchDepth - 1); return done(result);
                }
                @Override public boolean commitText(CharSequence text, int cursor) { return active() && done(super.commitText(text, cursor)); }
                @Override public boolean setComposingText(CharSequence text, int cursor) { return active() && done(super.setComposingText(text, cursor)); }
                @Override public boolean setComposingRegion(int start, int end) { return active() && done(super.setComposingRegion(start, end)); }
                @Override public boolean finishComposingText() { return active() && done(super.finishComposingText()); }
                @Override public boolean deleteSurroundingText(int before, int after) { return active() && done(super.deleteSurroundingText(before, after)); }
                @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) { return active() && done(super.deleteSurroundingTextInCodePoints(before, after)); }
                @Override public boolean setSelection(int start, int end) { return active() && done(super.setSelection(start, end)); }
                @Override public boolean sendKeyEvent(KeyEvent event) { return active() && done(super.sendKeyEvent(event)); }
                @Override public boolean performContextMenuAction(int id) { return active() && done(super.performContextMenuAction(id)); }
            };
        }
        @Override public boolean onKeyUp(int code, KeyEvent event) {
            boolean result = super.onKeyUp(code, event); publishEditor(); return result;
        }
    }

    public String gpuiClipboardRead() {
        ClipboardManager clipboard = (ClipboardManager)getSystemService(CLIPBOARD_SERVICE);
        ClipData clip = clipboard.getPrimaryClip();
        return clip == null || clip.getItemCount() == 0 ? "" : clip.getItemAt(0).coerceToText(this).toString();
    }
    public void gpuiClipboardWrite(String text) {
        runOnUiThread(() -> ((ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText("OptChat", text)));
    }
    public void gpuiOpenUrl(String url) {
        runOnUiThread(() -> { try { startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url))); }
            catch (android.content.ActivityNotFoundException ignored) {} });
    }

    public boolean prepareMicrophone() {
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED)
            return true;
        runOnUiThread(() -> requestPermissions(new String[] { android.Manifest.permission.RECORD_AUDIO }, 1));
        return false;
    }

    private SecretKey tokenKey() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
        SecretKey key = (SecretKey)store.getKey(KEY_ALIAS, null);
        if (key == null) {
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build());
            key = generator.generateKey();
        }
        return key;
    }
    private SharedPreferences tokenPrefs() { return getSharedPreferences("encrypted_tokens", MODE_PRIVATE); }
    private String tokenId(String key) throws Exception {
        byte[] hash = MessageDigest.getInstance("SHA-256").digest(key.getBytes(StandardCharsets.UTF_8));
        return Base64.encodeToString(hash, Base64.NO_WRAP | Base64.URL_SAFE);
    }
    public synchronized String getToken(String key) {
        try {
            String encoded = tokenPrefs().getString(tokenId(key), null); if (encoded == null) return null;
            ByteBuffer buffer = ByteBuffer.wrap(Base64.decode(encoded, Base64.NO_WRAP));
            int ivLength = buffer.get() & 255; if (ivLength != 12 || buffer.remaining() < ivLength + 16) return null;
            byte[] iv = new byte[ivLength]; buffer.get(iv); byte[] ciphertext = new byte[buffer.remaining()]; buffer.get(ciphertext);
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, tokenKey(), new GCMParameterSpec(128, iv));
            cipher.updateAAD(key.getBytes(StandardCharsets.UTF_8));
            return new String(cipher.doFinal(ciphertext), StandardCharsets.UTF_8);
        } catch (Exception error) { return null; }
    }
    public synchronized boolean setToken(String key, String token) {
        try {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, tokenKey());
            cipher.updateAAD(key.getBytes(StandardCharsets.UTF_8)); byte[] encrypted = cipher.doFinal(token.getBytes(StandardCharsets.UTF_8));
            byte[] iv = cipher.getIV(); ByteBuffer value = ByteBuffer.allocate(1 + iv.length + encrypted.length);
            value.put((byte)iv.length).put(iv).put(encrypted);
            return tokenPrefs().edit().putString(tokenId(key), Base64.encodeToString(value.array(), Base64.NO_WRAP)).commit();
        } catch (Exception error) { return false; }
    }
    public synchronized void deleteToken(String key) {
        try { tokenPrefs().edit().remove(tokenId(key)).commit(); } catch (Exception ignored) {}
    }
}
