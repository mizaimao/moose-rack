package io.github.mizaimao.moose_rack

import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewFeature
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
    /**
     * Turn off wry's own Back handling.
     *
     * It asks the WebView `canGoBack()` and finishes the activity when the
     * answer is no. This app is one page that never navigates, so the answer is
     * always no and every press quit to the launcher — from inside a platform,
     * a collection, the lightbox, anywhere.
     *
     * Pushing a history entry from JavaScript does not help, and that is worth
     * recording because it looks like it should: `history.pushState` moves
     * `history.length` to 2, but same-document entries do not reach the
     * WebView's back-forward list, so `canGoBack()` stays false and the page
     * never sees a `popstate`. Measured on the device over the DevTools
     * protocol — the press arrived, the counter stayed at zero.
     */
    override val handleBackNavigation = false

    /**
     * Every webview this activity has been given, oldest first.
     *
     * Not one reference, because there is more than one. Settings is a real
     * second window — `WebviewWindowBuilder` with its own settings.html — and
     * on Android that is a second WebView stacked in this same activity rather
     * than a separate task. Holding only the newest meant Back asked whichever
     * page happened to be built last, and holding only the first meant Back
     * asked the page underneath the one you were looking at. Either way
     * Settings could not be closed with Back.
     */
    private val webViews = mutableListOf<WebView>()

    /**
     * The game handed to another app, and when.
     *
     * There is no other way to know how long a game was played. RetroArch is a
     * separate app started by an Intent; `startActivity` returns as soon as the
     * request is accepted and nothing reports back when the emulator stops. What
     * *is* observable is this activity coming back to the front, which is what
     * happens when the game ends — so the two ends of the measurement are the
     * Intent going out and the window regaining focus.
     *
     * `elapsedRealtime` rather than the wall clock, which a user can change and
     * which would then produce a negative play time.
     */
    private var playingRomId = -1L
    private var playingSince = 0L

    /** Launcher for the folder picker, and where to send the answer back. */
    private lateinit var folderPicker: ActivityResultLauncher<Uri?>
    private var folderPickerTarget: String? = null

    override fun onWebViewCreate(webView: WebView) {
        webViews.add(webView)
        // Before anything can focus it. See stopFocusHighlight: after the view
        // has been focused once, the highlight is already installed and turning
        // this off does not take it away again.
        stopFocusHighlight(webView)
        // Paint the WebView itself, not just the page inside it.
        //
        // A WebView starts white and composites the page over it, and this page
        // is not opaque everywhere — with a backdrop running `html` and `body`
        // are transparent by design and the canvas provides the colour. Whatever
        // the canvas does not cover would show that white.
        webView.setBackgroundColor(Color.parseColor("#14161A"))
        stopDarkening(webView)
        // The page's only way to reach Android. See Bridge.
        webView.addJavascriptInterface(Bridge(), "MooseAndroid")
    }

    /**
     * The handful of Android things the settings page needs to reach.
     *
     * A JavaScript interface rather than a Tauri command, because a Tauri
     * command runs in Rust and Rust has no way to get at the Android context —
     * Tauri does not expose it (tauri-apps/tauri#13267). Kotlin has it for
     * free, so the shortest honest path from a button in the page to an Intent
     * is this.
     *
     * Only this app's own local pages run in this webview, so there is no third
     * party to expose it to.
     */
    private inner class Bridge {
        /** Whether the app can read the ES-DE folders. */
        @JavascriptInterface
        fun hasAllFilesAccess(): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.R ||
                Environment.isExternalStorageManager()

        /**
         * Show the system switch for All files access.
         *
         * There is no dialog an app can raise for this one and no way to grant
         * it in-process; the switch lives in system settings and the most any
         * app can do is open that screen. Without a button for it, dismissing
         * the prompt at launch left no way back — which is exactly what
         * happened.
         */
        @JavascriptInterface
        fun openAllFilesAccess() {
            runOnUiThread { openAllFilesScreen() }
        }

        /**
         * Whether RetroArch is installed, and which build.
         *
         * On Android RetroArch is a package, not a path — there is nothing to
         * browse for and nothing to point at. Either the app is on the device
         * or it is not, and the settings page should say which rather than
         * offering a file picker that cannot mean anything.
         *
         * Both ABIs are checked because a device may carry either.
         */
        /**
         * Open the system folder picker and hand the answer back to the page.
         *
         * Android has no directory dialog an app can call and await; it has an
         * activity that returns a result later. So this returns nothing, and
         * the answer arrives at `window.__folderPicked(target, path)` — the
         * target being whichever field asked, so two pickers cannot be
         * confused.
         */
        @JavascriptInterface
        fun pickFolder(target: String) {
            runOnUiThread {
                folderPickerTarget = target
                try {
                    folderPicker.launch(null)
                } catch (e: Exception) {
                    folderPickerTarget = null
                }
            }
        }

        /**
         * Start a game in RetroArch.
         *
         * Android does not run emulators, it starts apps. RetroArch is another
         * package and the only way in is an explicit Intent to
         * `RetroActivityFuture` carrying three string extras, which is exactly
         * what ES-DE sends — its `es_systems.xml` entries expand to
         *
         *     %EMULATOR_RETROARCH%
         *       %EXTRA_CONFIGFILE%=/storage/emulated/0/Android/data/<pkg>/files/retroarch.cfg
         *       %EXTRA_LIBRETRO%=/data/data/<pkg>/cores/<core>_libretro_android.so
         *       %EXTRA_ROM%=<rom>
         *
         * and this builds the same thing. The paths are RetroArch's own, not
         * ours: its cores are in its private directory, which this app cannot
         * read, and its config is under `Android/data`, which Android 11 closed
         * to everyone. Neither has to be readable here — they are handed over
         * as strings and RetroArch opens them itself.
         *
         * Takes the whole plan rather than a component and a core because
         * *which* RetroArch is installed is a question only this side can
         * answer, and the answer changes both paths. `com.retroarch.aarch64` on
         * every handheld worth the name, `com.retroarch` on the rest; the plan
         * lists both and the first one that is really here wins.
         *
         * No flags. `RetroActivityFuture` is `launchMode="singleInstance"`, so
         * it gets its own task whatever we ask for, and Back comes home.
         *
         * Returns an empty string when the game is on its way, and something to
         * show the user when it is not.
         */
        /**
         * Where to put files RetroArch has to read.
         *
         * This app's *private* directory is the natural place and the wrong
         * one: nothing else on the device can open it. This is the external
         * one — `Android/data/<us>/files` — which RetroArch reads because it
         * targets SDK 28 and is still on legacy storage. Measured, not assumed:
         * a config written here was accepted, and the launch logged
         * `[ENV] Config file: ...` pointing back at it.
         *
         * Asked of the framework rather than spelled out, because the path
         * differs on a device with the app on a card.
         */
        @JavascriptInterface
        fun externalFilesDir(): String = getExternalFilesDir(null)?.absolutePath ?: ""

        /**
         * Hand a file or a link to whatever app on the device handles it.
         *
         * Three things in the info pane could not work in this webview and all
         * three fail differently, which is why they end up in one place:
         *
         * * **A manual is a PDF**, and Android's webview has no PDF viewer —
         *   `navigator.pdfViewerEnabled` is false, where every desktop engine
         *   this app runs on says true.
         * * **A video hangs.** The bytes are served: `fetch` on the same URL
         *   answers 206 with a byte range and the right content type. But a
         *   `<video>` element sits at `readyState 0` and `networkState 2`
         *   forever, because Chromium's media loader does not go through the
         *   same path as `fetch` for a custom scheme.
         * * **A trailer is an ordinary link**, and Tauri's opener tries to run
         *   a desktop command for it — the error is `No such file or directory
         *   (os error 2)`, which is it failing to find `xdg-open`.
         *
         * A `content://` URI rather than a `file://` one: passing the latter to
         * another app has thrown `FileUriExposedException` since Android 7.
         * The grant is per URI and read-only, and it lasts as long as the
         * activity it was given to.
         *
         * Returns an empty string when something took it, and a reason when
         * nothing did — a device with no PDF reader is a real answer, not a
         * crash.
         */
        @JavascriptInterface
        fun openExternal(target: String, mime: String): String {
            if (target.isEmpty()) return "nothing to open"
            val intent = Intent(Intent.ACTION_VIEW)
            if (target.startsWith("http://") || target.startsWith("https://")) {
                intent.data = Uri.parse(target)
            } else {
                val file = java.io.File(target)
                if (!file.isFile) return "that file is not on this device any more"
                val uri =
                    try {
                        androidx.core.content.FileProvider.getUriForFile(
                            this@MainActivity,
                            "$packageName.fileprovider",
                            file,
                        )
                    } catch (e: Exception) {
                        return "could not share that file: $e"
                    }
                intent.setDataAndType(uri, mime.ifEmpty { "*/*" })
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            // Its own task, so coming back lands here rather than inside a
            // video player showing this app's last screen.
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (packageManager.resolveActivity(intent, 0) == null) {
                return "nothing on this device opens that"
            }
            runOnUiThread {
                try {
                    startActivity(intent)
                } catch (e: Exception) {
                }
            }
            return ""
        }

        @JavascriptInterface
        fun startEmulator(planJson: String): String {
            val plan =
                try {
                    org.json.JSONObject(planJson)
                } catch (e: Exception) {
                    return "could not read the launch plan"
                }
            val rom = plan.optString("rom")
            if (rom.isEmpty()) return "the launch plan has no ROM path"
            val candidates = plan.optJSONArray("candidates")
            if (candidates == null || candidates.length() == 0) {
                return "nothing is listed that could run this"
            }

            val looked = LinkedHashSet<String>()
            for (i in 0 until candidates.length()) {
                val c = candidates.optJSONObject(i) ?: continue
                val component = c.optString("component")
                val slash = component.indexOf('/')
                if (slash <= 0) continue
                val pkg = component.substring(0, slash)
                val rest = component.substring(slash + 1)
                // ES-DE writes the activity relative to its package when the two
                // share a prefix. An Intent wants it whole.
                val activity = if (rest.startsWith(".")) pkg + rest else rest
                looked.add(pkg)

                val intent = Intent().setComponent(ComponentName(pkg, activity))
                intent.putExtra("ROM", rom)
                val core = c.optString("core_file")
                if (core.isNotEmpty()) {
                    intent.putExtra("LIBRETRO", "/data/data/$pkg/cores/$core")
                    // Ours when the backend managed to build one, RetroArch's
                    // own when it did not. `CONFIGFILE` is the *whole* config —
                    // anything it omits falls back to RetroArch's defaults
                    // rather than to the user's settings — so the generated one
                    // is their file with our changes merged into it, never a
                    // fragment on its own. See `android_config` in lib.rs.
                    val ours = plan.optString("config")
                    intent.putExtra(
                        "CONFIGFILE",
                        if (ours.isNotEmpty()) ours
                        else "/storage/emulated/0/Android/data/$pkg/files/retroarch.cfg",
                    )
                }

                // Asked of the component, not the package. A package can be
                // installed while the activity we want is not the one it
                // exports, and that is an ActivityNotFoundException on a
                // background thread — a crash rather than a message. This is
                // also the visibility check: since Android 11 an activity in a
                // package the manifest does not declare in `<queries>` is
                // invisible, and this reports it as missing, which is the same
                // answer the launch would get.
                try {
                    packageManager.getActivityInfo(intent.component!!, 0)
                } catch (e: PackageManager.NameNotFoundException) {
                    continue
                }

                // On the UI thread, like every other `startActivity` here.
                // Nothing is awaited: the activity is known to exist by now, and
                // a game takes seconds to appear, so there is nothing useful to
                // report back that the screen will not say first.
                playingRomId = plan.optLong("id", -1L)
                playingSince = android.os.SystemClock.elapsedRealtime()
                runOnUiThread { startActivity(intent) }
                return ""
            }
            return "RetroArch is not installed — looked for " + looked.joinToString(", ")
        }

        @JavascriptInterface
        fun retroArchPackage(): String {
            for (pkg in arrayOf("com.retroarch.aarch64", "com.retroarch")) {
                try {
                    packageManager.getPackageInfo(pkg, 0)
                    return pkg
                } catch (e: Exception) {
                    // Not installed under that name; try the next.
                }
            }
            return ""
        }
    }

    /**
     * The page the user is actually looking at.
     *
     * Newest first, skipping any that have been detached — a closed settings
     * window leaves its WebView in the list but off the view tree, and asking a
     * dead one would silently answer "no" and quit the app.
     */
    private fun topWebView(): WebView? =
        webViews.lastOrNull { it.isAttachedToWindow && it.isShown }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        goFullScreen()
        askForFilesOnce()

        // Registered here because a result launcher may only be created before
        // the activity is STARTED. The page asks for a folder later, by name,
        // and the name comes back with the answer so two pickers cannot be
        // confused for each other.
        folderPicker =
            registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
                val target = folderPickerTarget ?: return@registerForActivityResult
                folderPickerTarget = null
                val path = uri?.let { treeUriToPath(it) } ?: ""
                val js = "window.__folderPicked && window.__folderPicked(" +
                    org.json.JSONObject.quote(target) + "," +
                    org.json.JSONObject.quote(path) + ")"
                topWebView()?.evaluateJavascript(js, null)
            }

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                val wv = topWebView()
                if (wv == null) {
                    // Pressed before the page exists. Nothing can have an
                    // opinion yet, so behave like any other app.
                    quit()
                    return
                }
                // The page answers "true" when it consumed the press — it
                // closed the lightbox, the help panel, or walked one level up.
                // Anything else, including a page that has not finished
                // loading and so has no such function, means there was nowhere
                // to go and Back should leave.
                //
                // Asynchronous, because `evaluateJavascript` is. The press is
                // already consumed by the time the answer arrives, so quitting
                // happens in the callback rather than after it.
                wv.evaluateJavascript(ASK) { handled ->
                    if (handled != "true") quit()
                }
            }

            /**
             * Hand the press back to the system.
             *
             * Disabling this callback first is what stops it catching its own
             * re-dispatch and looping forever.
             */
            private fun quit() {
                isEnabled = false
                onBackPressedDispatcher.onBackPressed()
                isEnabled = true
            }
        })
    }

    /**
     * Hide the status bar and the gesture bar.
     *
     * This is a games launcher on a handheld; the clock, the battery and the
     * wifi bars belong to a phone. They also sat *over* the app rather than
     * above it — `enableEdgeToEdge` draws behind them — so the first tab was
     * underneath the clock and the last row of the list underneath the gesture
     * pill.
     *
     * BEHAVIOUR_SHOW_TRANSIENT_BARS_BY_SWIPE rather than hiding them outright:
     * a swipe from the edge brings them back for a few seconds and then they
     * leave again, so the time and the battery are still reachable while
     * nothing is permanently covering the library.
     *
     * Re-applied on focus because Android puts them back after a dialog, the
     * recents switcher, or the folder picker returning.
     */
    private fun goFullScreen() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior =
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            goFullScreen()
            paintWebViews()
            reportGameFinished()
        }
    }

    /**
     * Tell the page the game is over, and how long it lasted.
     *
     * Only once per launch: focus comes back for a dialog, the recents
     * switcher and the folder picker too, and each of those would otherwise be
     * reported as a finished game. Clearing the id first is what makes it once.
     *
     * The page does the rest — recording the play and pushing saves — because
     * that is Rust's work and this side cannot reach it.
     */
    private fun reportGameFinished() {
        if (playingRomId < 0) return
        val id = playingRomId
        val seconds = (android.os.SystemClock.elapsedRealtime() - playingSince) / 1000
        playingRomId = -1L
        val js = "window.__gameFinished && window.__gameFinished($id, $seconds)"
        topWebView()?.evaluateJavascript(js, null)
    }

    /**
     * Make every webview opaque, again.
     *
     * Setting this in `onWebViewCreate` is too early: wry configures the view
     * after handing it over, and Tauri supports transparent windows, so it puts
     * the background back to nothing. A transparent WebView over a white
     * default is white anywhere the page does not paint, and with a backdrop
     * running the page deliberately does not paint.
     */
    private fun paintWebViews() {
        val bg = Color.parseColor("#14161A")
        for (wv in webViews) {
            // An opaque colour is what makes a WebView opaque; `isOpaque` is
            // derived and read-only.
            wv.setBackgroundColor(bg)
            stopDarkening(wv)
            stopFocusHighlight(wv)
        }
    }

    /**
     * Stop Android painting its own focus highlight over the whole page.
     *
     * **This is the tint.** A flat wash over everything, arriving the moment the
     * stick or the d-pad is touched and never leaving. It is documented at
     * length in docs/tint.md; the short version:
     *
     * Since Oreo, a focused view that has no focus state of its own gets one
     * drawn for it — `?android:attr/selectableItemBackground`, a ripple, painted
     * over the view's whole bounds. It appears only once the window leaves touch
     * mode, and a d-pad or stick press is exactly what does that; a fresh launch
     * is in touch mode, which is why the first frames look right. Re-entering
     * touch mode does not undo it, because the drawable is only chosen when
     * focus changes.
     *
     * The colour is arithmetic rather than a guess. `colorControlHighlight` in a
     * night-mode Material theme is #33FFFFFF — twenty per cent of white — and
     * `RippleBackground.FOCUSED_ALPHA` is 0.6, so what lands is twelve per cent
     * of white. Measured on the device: the page painted #000000 came out
     * #1F1F1F, and 0.12 * 255 = 30.6, which is 0x1F.
     *
     * Everything that made this hard to find follows from *where* it is drawn.
     * It is a View foreground, so it goes on after Chromium has finished: the
     * page cannot see it, `getComputedStyle` reports the colour the stylesheet
     * asked for, and no background anywhere in the app — page, webview, window,
     * theme — is underneath it. It is inside the app's own raster, so
     * SurfaceFlinger reports the layer as opaque with an identity colour
     * transform. And it covers the view's bounds, so it lands on the canvas and
     * the artwork too, at which point the whole screen has a floor: measured
     * across 1920x1080, one pixel out of 2,073,600 was darker than #1F1F1F.
     *
     * The one number that ever argued against "something white on top" was
     * #FF0000 coming out #EA3D31, which no white overlay produces. That was the
     * capture, not the app: `screencap` reads back in Display-P3, and twelve per
     * cent of white over red, converted to P3, is (234, 61, 49) exactly.
     *
     * There is nothing in this app for the highlight to be useful to — one
     * webview, filling the window, and every focus ring the user sees is drawn
     * by the page.
     */
    private fun stopFocusHighlight(wv: WebView) {
        // API 26. Below it there is no default focus highlight to turn off.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            wv.defaultFocusHighlightEnabled = false
        }
    }

    /**
     * Stop the WebView rewriting this page's colours.
     *
     * The device is in night mode, so the WebView may apply its own darkening
     * pass on top of the page: it decides dark backgrounds are too dark and
     * lifts them toward a Material surface colour. This app is already dark and
     * did not ask.
     *
     * Housekeeping rather than a fix for anything observed. It was set while
     * chasing the tint, on the theory that the wash was this pass; it was not
     * (see stopFocusHighlight), and turning it off changed nothing. It stays
     * because it is the correct setting for a page that themes itself, and
     * because leaving it at the default invites the question again.
     *
     * Through androidx.webkit rather than the platform, which is the part worth
     * remembering. The platform switches were both set and neither took: the
     * newer one reads back false as asked, and the older one is a no-op at this
     * target and still reads AUTO. This WebView is AOSP Chromium 109, baked into
     * the ROM and not updatable; `WebSettingsCompat` routes to whatever the
     * installed WebView actually implements.
     *
     * Two APIs because the name changed: `isAlgorithmicDarkeningAllowed` from
     * 33, `forceDark` before it.
     */
    @Suppress("DEPRECATION")
    private fun stopDarkening(wv: WebView) {
        try {
            if (WebViewFeature.isFeatureSupported(WebViewFeature.ALGORITHMIC_DARKENING)) {
                WebSettingsCompat.setAlgorithmicDarkeningAllowed(wv.settings, false)
            }
        } catch (e: Exception) {
        }
        try {
            @Suppress("DEPRECATION")
            if (WebViewFeature.isFeatureSupported(WebViewFeature.FORCE_DARK)) {
                WebSettingsCompat.setForceDark(wv.settings, WebSettingsCompat.FORCE_DARK_OFF)
            }
        } catch (e: Exception) {
        }
    }

    /**
     * Offer the All files access toggle whenever it is not granted.
     *
     * The ES-DE library lives in ordinary folders — /storage/emulated/0/ES-DE
     * and /storage/emulated/0/ROMs — and since Android 11 no ordinary
     * permission opens them. MANAGE_EXTERNAL_STORAGE does, and it is granted by
     * a switch in system settings rather than by a dialog an app can raise, so
     * the most an app can do is take you to the switch.
     *
     * Every launch until it is granted, which is what the other frontends on
     * this kind of device do — ES-DE included. An earlier version asked once
     * and remembered, on the reasoning that throwing someone out to a settings
     * screen twice is rude; that was the wrong trade. Without this permission a
     * frontend cannot see the library at all, so a single missed prompt leaves
     * an app that looks broken and says nothing, and there is no in-app control
     * to find your way back to.
     *
     * The check is the grant itself, so it stops on its own the moment the
     * switch is on. Nothing is remembered and there is no state to get stuck.
     */
    private fun askForFilesOnce() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
        if (Environment.isExternalStorageManager()) return
        openAllFilesScreen()
    }

    /**
     * Open the All files access screen, from launch or from the settings page.
     *
     * Wrapped: the per-app screen is missing on some builds, and a crash would
     * be a far worse trade than a permission nobody was offered. The generic
     * list is the fallback, and giving up is fine.
     */
    private fun openAllFilesScreen() {
        try {
            startActivity(
                Intent(
                    Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                    Uri.parse("package:$packageName"),
                )
            )
        } catch (e: Exception) {
            try {
                startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
            } catch (e: Exception) {
                // No settings screen to offer. The app works without it.
            }
        }
    }

    /**
     * Turn what the folder picker hands back into a path the app can open.
     *
     * The picker answers with a tree URI — `content://com.android.externalstorage
     * .documents/tree/primary%3AGames%2FROMs` — and every path in this app is a
     * filesystem path. With All files access granted the two describe the same
     * place, so the URI is unwrapped rather than carried around: `primary` is
     * shared storage, and anything else is a card, mounted under /storage by
     * its volume id.
     *
     * Returns empty when the shape is not recognised, and the page says it
     * could not read that folder rather than storing a path that opens nothing.
     */
    private fun treeUriToPath(uri: Uri): String {
        val id = android.provider.DocumentsContract.getTreeDocumentId(uri) ?: return ""
        val parts = id.split(':', limit = 2)
        val volume = parts.getOrNull(0) ?: return ""
        val rest = parts.getOrNull(1).orEmpty()
        val root = if (volume.equals("primary", true)) {
            Environment.getExternalStorageDirectory().absolutePath
        } else {
            "/storage/$volume"
        }
        return if (rest.isEmpty()) root else "$root/$rest"
    }

    private companion object {
        const val ASK = "window.__androidBack ? window.__androidBack() : false"
    }
}
